extern crate actix_web;
extern crate actix_service;
extern crate listenfd;
extern crate redis;
extern crate uuid;
extern crate log;
extern crate simple_logger;
extern crate rand;

use std::convert::TryFrom;
use std::collections::HashSet;
use actix_web::{http, web, App, HttpRequest, HttpResponse, HttpServer, Result};
use http::StatusCode;
use serde::Serialize;
use listenfd::ListenFd;
use log::{debug, error};

mod components;
mod middleware;

use components::InfocomError;
use components::memory::{MemoryMap, ZByte, ZWord, ZValue};
use components::session::Session;
use components::text::{Decoder,Encoder};
use components::object_table::ObjectTable;
use components::state::{ FrameStack, Routine };
use components::instruction;
use components::interface::{ Curses, Interface };

async fn new_session(_req: HttpRequest) -> HttpResponse {
    let s = Session::new().unwrap();
    HttpResponse::Ok()
        .cookie(http::Cookie::build("session", format!("{}", &s.id)).finish())
        .json(s)
}

async fn get_session(req: HttpRequest) -> HttpResponse {
    let id = req.headers().get("x-session").unwrap().to_str().unwrap();
    match Session::try_from(id) {
        Ok(session) => {
            HttpResponse::Ok().json(session)
        },
        Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
    }
}

async fn new_story(req: HttpRequest, data: web::Bytes) -> HttpResponse {
     let name = req.match_info().get("name").unwrap();
     let id = req.headers().get("x-session").unwrap().to_str().unwrap();
     match Session::try_from(id) {
         Ok(mut session) => {
            if let Ok(mem) = MemoryMap::try_from(data.to_vec()) {
                if let Err(e) = session.add_story(String::from(name), mem) {
                    error!("{}", e);
                    HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                } else {
                    HttpResponse::Ok().json(session)
                }
            } else {
                HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).finish()
            }
        },
        Err(e) => HttpResponse::build(StatusCode::NOT_FOUND).body(e.to_string())
     }
}

fn error(function: &str, error: InfocomError, address: usize) -> Result<HttpResponse> {
    error!("{}", error);
    error!("{} at ${:06x} FAILED", function, address);
    Ok(HttpResponse::build(StatusCode::BAD_REQUEST).body(format!("Invalid {} at ${:06x}", function, address)))
}

fn load_memory(id: &str, name: &str) -> Result<MemoryMap, InfocomError> {
    Session::try_from(id)?.load(name)
}

fn read_from_memory<T>(req: HttpRequest, address: usize) -> Result<T, InfocomError>
where 
    T: ZValue
{
    let name = req.match_info().get("name").unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        let mem = load_memory(id.to_str().unwrap(), name)?;
        let mut values = Vec::<u8>::new();
        let bytes = T::size();
        for i in 0..bytes {
            match mem.get_byte(address + i) {
                Ok(value) => values.push(value),
                Err(e) => return Err(e)
            }
        }
        
        Ok(T::new(&values))
    } else {
        Err(InfocomError::API(format!("Missing session id")))
    }
}

async fn read_byte(req: HttpRequest) -> Result<HttpResponse> {
    let address: usize = req.match_info().get("address").unwrap().parse().unwrap();
    let value:Result<ZByte, InfocomError> = read_from_memory(req, address);
    match value {
        Ok(v) => Ok(HttpResponse::Ok().json(v)),
        Err(e) => error("read_byte", e, address)
    }
}

async fn read_word(req: HttpRequest) -> Result<HttpResponse> {
    let address: usize = req.match_info().get("address").unwrap().parse().unwrap();
    let value:Result<ZWord, InfocomError> = read_from_memory(req, address);
    match value {
        Ok(v) => Ok(HttpResponse::Ok().json(v)),
        Err(e) => error("read_byte", e, address)
    }    
}
fn type_from_values(values: &[u8]) -> &str {
    match values.len() {
        1 => "byte",
        2 => "word",
        _ => "unknown"
    }      
}

fn write_to_memory(req: HttpRequest, values: &[u8]) -> Result<HttpResponse> {
    let address: usize = req.match_info().get("address").unwrap().parse().unwrap();
    let func = &format!("write_{}", type_from_values(values));
    let name = req.match_info().get("name").unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        let mut index = address;
                        for value in values {
                            match mem.set_byte(index, *value) {
                                Ok(_) => {
                                    index = index + 1;
                                },
                                Err(e) => return error(func, e, address)
                            }
                        }
                        match session.save(name, mem) {
                            Ok(_) => {
                                debug!("{}: ${:?} to ${:06x}", func, values, address);
                                Ok(HttpResponse::Ok().finish()) 
                            },
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        }
                    },
                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                }
            },
            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
        }
    } else {
        Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn write_byte(req: HttpRequest) -> Result<HttpResponse> {
    let values: &[u8] = &vec![req.match_info().get("value").unwrap().parse().unwrap()];
    write_to_memory(req, values)
}

async fn write_word(req: HttpRequest) -> Result<HttpResponse> {
    let value: u16 = req.match_info().get("value").unwrap().parse().unwrap();
    let values = &vec![(value >> 8 & 0xFF) as u8, (value & 0xFF) as u8];
    write_to_memory(req, values)
}

async fn read_text(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let address:usize = req.match_info().get("address").unwrap().parse().unwrap();
    match req.headers().get("X-Session") {
        Some(id) => {
            match load_memory(id.to_str().unwrap(), name) {
                Ok(mem) => {
                    match Decoder::new(&mem) {
                        Ok(decoder) => match decoder.decode(address) {
                            Ok(text) => Ok(HttpResponse::Ok().json(text)),
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        },
                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                    }
                },
                Err(_) => Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
            }
        },
        None => {
            Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
        }
    }
}

async fn encode_text(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let string = req.match_info().get("string").unwrap();
    match req.headers().get("X-Session") {
        Some(id) => {
            match load_memory(id.to_str().unwrap(), name) {
                Ok(mem) => {
                    match Encoder::new(&mem) {
                        Ok(encoder) => match encoder.encode(string) {
                            Ok(text) => Ok(HttpResponse::Ok().json(text)),
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        },
                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                    }
                },
                Err(_) => Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
            }
        },
        None => {
            Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
        }
    }
}

async fn get_object(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    match req.headers().get("X-Session") {
        Some(id) => {
            match load_memory(id.to_str().unwrap(), name) {
                Ok(mut mem) => {
                    match ObjectTable::new(&mut mem) {
                        Ok(ot) => match ot.get_object(&mem, number) {
                            Ok(obj) => Ok(HttpResponse::Ok().json(obj)),
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        },
                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                    }
                },
                Err(_) => Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
            }
        },
        None => {
            Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
        }
    }
}

async fn has_object_attribute(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    let attribute:usize = req.match_info().get("attribute").unwrap().parse().unwrap();
    match req.headers().get("X-Session") {
        Some(id) => match load_memory(id.to_str().unwrap(), name) {
                        Ok(mut mem) => {
                            match ObjectTable::new(&mut mem) {
                                Ok(ot) => match ot.has_attribute(&mem, number, attribute) {
                                    Ok(r) => Ok(HttpResponse::Ok().json(r)),
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                },
                                Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                            }
                        },
                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                    },
        None => Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn set_object_attribute(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    let attribute:usize = req.match_info().get("attribute").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match FrameStack::new(&mut mem) {
                            Ok(mut f) => {
                                match ObjectTable::new(f.get_memory()) {
                                    Ok(mut ot) => match ot.set_attribute(&mut f, number, attribute) {
                                        Ok(o) => match session.save(name, mem) {
                                            Ok(_) => {
                                                Ok(HttpResponse::Ok().json(o)) 
                                            },
                                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                        },
                                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                    },
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                }
                            },
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        }
                    },
                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                }
            },
            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
        }
    } else {
        Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn clear_object_attribute(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    let attribute:usize = req.match_info().get("attribute").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match FrameStack::new(&mut mem) {
                            Ok(mut f) => {
                                match ObjectTable::new(f.get_memory()) {
                                    Ok(mut ot) => match ot.clear_attribute(&mut f, number, attribute) {
                                        Ok(o) => match session.save(name, mem) {
                                            Ok(_) => {
                                                Ok(HttpResponse::Ok().json(o)) 
                                            },
                                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                        },
                                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                    },
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                }
                            },
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        }
                    },
                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                }
            },
            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
        }
    } else {
        Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn get_object_property(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    let property:usize = req.match_info().get("property").unwrap().parse().unwrap();
    match req.headers().get("X-Session") {
        Some(id) => match load_memory(id.to_str().unwrap(), name) {
                        Ok(mut mem) => {
                            match ObjectTable::new(&mut mem) {
                                Ok(ot) => match ot.get_property_value(&mem, number, property) {
                                    Ok(data) => Ok(HttpResponse::Ok().json(data)),
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                },
                                Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                            }
                        },
                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                    },
        None => Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn put_object_property(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    let property:usize = req.match_info().get("property").unwrap().parse().unwrap();
    let value:u16 = req.match_info().get("value").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match FrameStack::new(&mut mem) {
                            Ok(mut f) => {
                                match ObjectTable::new(f.get_memory()) {
                                    Ok(mut ot) => match ot.put_property_data(&mut f, number, property, value) {
                                        Ok(o) => match session.save(name, mem) {
                                            Ok(_) => Ok(HttpResponse::Ok().json(o)), 
                                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                        },
                                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                    },
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                }
                            },
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        }
                    },
                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                }
            },
            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
        }
    } else { 
        Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn remove_object(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match FrameStack::new(&mut mem) {
                            Ok(mut f) => {
                                match ObjectTable::new(f.get_memory()) {
                                    Ok(mut ot) => match ot.remove_object(&mut f, number) {
                                        Ok(o) => match session.save(name, mem) {
                                            Ok(_) => Ok(HttpResponse::Ok().json(o)), 
                                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                        },
                                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                    },
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                }
                            },
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        }
                    },
                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                }
            },
            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
        }
    } else { 
        Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

async fn insert_object(req: HttpRequest) -> Result<HttpResponse> {
    let name = req.match_info().get("name").unwrap();
    let parent:usize = req.match_info().get("parent").unwrap().parse().unwrap();
    let number:usize = req.match_info().get("number").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match FrameStack::new(&mut mem) {
                            Ok(mut f) => {
                                match ObjectTable::new(f.get_memory()) {
                                    Ok(mut ot) => match ot.insert_object(&mut f, number, parent) {
                                        Ok(o) => match session.save(name, mem) {
                                            Ok(_) => Ok(HttpResponse::Ok().json(o)), 
                                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                        },
                                        Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                    },
                                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                                }
                            },
                            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                        }
                    },
                    Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
                }
            },
            Err(e) => Ok(HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()))
        }
    } else { 
        Ok(HttpResponse::build(StatusCode::NOT_FOUND).finish())
    }
}

#[derive(Serialize, Debug)]
struct ObjectTreeEntry {
    number: u16,
    short_name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<ObjectTreeEntry>
}

fn build_tree_entry(mem: &MemoryMap, ot: &ObjectTable, number: usize) -> ObjectTreeEntry {
    let o = ot.get_object(mem, number).unwrap();
    let mut c = o.get_child();
    let mut children = Vec::new();

    while c != 0 {
        let o_c = ot.get_object(mem, c as usize).unwrap();
        children.push(build_tree_entry(mem, ot, c as usize));
        c = o_c.get_sibling();
    }

    ObjectTreeEntry { number: number as u16, short_name: o.get_short_name(), children }
}

async fn object_tree(req: HttpRequest) -> HttpResponse {
    let name = req.match_info().get("name").unwrap();
    let end:usize = req.match_info().get("end").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        // Find all children of the root
                        let mut placed = HashSet::new();
                        let mut tree = Vec::new();
                        match ObjectTable::new(&mut mem) {
                            Ok(ot) => {
                                for i in 1..(end + 1) {
                                    if let Ok(o) = ot.get_object(&mem, i) {
                                        if !placed.contains(&i) && o.get_parent() == 0 {
                                            placed.insert(i);
                                            tree.push(build_tree_entry(&mem, &ot, i));
                                        }
                                    } else {
                                        // Premature end of object table?
                                        break;
                                    }
                                }
                                HttpResponse::Ok().json(tree)
                            },
                            Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                        }
                    },
                    Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                }
            },
            Err(e) => HttpResponse::build(StatusCode::NOT_FOUND).body(e.to_string())
        }
    } else {
        HttpResponse::build(StatusCode::NOT_FOUND).finish()
    }
}

// async fn instruction(req: HttpRequest) -> HttpResponse {
//     let name = req.match_info().get("name").unwrap();
//     let address:usize = req.match_info().get("address").unwrap().parse().unwrap();
//     if let Some(id) = req.headers().get("X-Session") {
//         match Session::try_from(id.to_str().unwrap()) {
//             Ok(mut session) => {
//                 match session.load(name) {
//                     Ok(mem) => {
//                         match instruction::decode_instruction(&f, address) {
//                             Ok(i) => HttpResponse::Ok().json(i),
//                             Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
//                         }
//                     },
//                     Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
//                 }
//             },
//             Err(e) => HttpResponse::build(StatusCode::NOT_FOUND).body(e.to_string())
//         }
//     } else {
//         HttpResponse::build(StatusCode::NOT_FOUND).finish()
//     }
// }

async fn get_routine(req: HttpRequest) -> HttpResponse {
    let name = req.match_info().get("name").unwrap();
    let address:usize = req.match_info().get("address").unwrap().parse().unwrap();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match Routine::new(&mut mem, address) {
                            Ok(r) => HttpResponse::Ok().json(r),
                            Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                        }
                    },
                    Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                }
            },
            Err(e) => HttpResponse::build(StatusCode::NOT_FOUND).body(e.to_string())
        }
    } else {
        HttpResponse::build(StatusCode::NOT_FOUND).finish()
    }
}

// async fn execute_instruction(req: HttpRequest) -> HttpResponse {
//     let name = req.match_info().get("name").unwrap();
//     let address:usize = req.match_info().get("address").unwrap().parse().unwrap();
//     if let Some(id) = req.headers().get("X-Session") {
//         match Session::try_from(id.to_str().unwrap()) {
//             Ok(mut session) => {
//                 match session.load(name) {
//                     Ok(mut mem) => {
//                         match instruction::decode_instruction(&mut mem, address) {
//                             Ok(mut i) => {
//                                 match FrameStack::new(&mut mem) {
//                                     Ok(mut f) => {
//                                         match i.execute(&mut f) {
//                                             Ok(r) => match session.save(name, mem) {
//                                                 Ok(_) => HttpResponse::Ok().json(r),
//                                                 Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
//                                             },
//                                             Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
//                                         }
//                                     },
//                                     Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
//                                 }
//                             },
//                             Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())                            
//                         } 
//                     },
//                     Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
//                 }
//             },
//             Err(e) => HttpResponse::build(StatusCode::NOT_FOUND).body(e.to_string())
//         }
//     } else {
//         HttpResponse::build(StatusCode::NOT_FOUND).finish()
//     }
// }

async fn run(req: HttpRequest) -> HttpResponse {
    let name = req.match_info().get("name").unwrap();
    let mut address:usize = req.match_info().get("address").unwrap().parse().unwrap();
    let mut interface = Curses::new();
    if let Some(id) = req.headers().get("X-Session") {
        match Session::try_from(id.to_str().unwrap()) {
            Ok(mut session) => {
                match session.load(name) {
                    Ok(mut mem) => {
                        match FrameStack::new(&mut mem) {
                            Ok(mut f) => {
                                loop {            
                                    match instruction::decode_instruction(&f, address) {
                                        Ok(mut i) => {
                                            match i.execute(&mut f, &mut interface) {
                                                Ok(r) => address = r,
                                                Err(e) => {
                                                    //interface.end();
                                                    match session.save(name, mem) {
                                                        Ok(_) => return HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string()),
                                                        Err(e2) => return HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(format!("{}\n{}", e.to_string(), e2.to_string()))
                                                    }
                                                }
                                            }
                                        },
                                        Err(e) => return HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())                            
                                    }
                                }
                            },
                            Err(e) => return HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                        } 
                    },
                    Err(e) => HttpResponse::build(StatusCode::INTERNAL_SERVER_ERROR).body(e.to_string())
                }
            },
            Err(e) => HttpResponse::build(StatusCode::NOT_FOUND).body(e.to_string())
        }
    } else {
        HttpResponse::build(StatusCode::NOT_FOUND).finish()
    }
}

use std::env;
use std::fs;
fn main() {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    
    let args: Vec<String> = env::args().collect();
    let filename = &args[1];

    let bytes = fs::read(filename).unwrap();
    let mut mem = MemoryMap::try_from(bytes).unwrap();
    let mut interface = Curses::new();
    let mut framestack = FrameStack::new(&mut mem).unwrap();
    let mut pc = framestack.pc();

    loop {
        let mut i = instruction::decode_instruction(&framestack, pc).unwrap();
        match i.execute(&mut framestack, &mut interface) {
            Ok(v) => pc = v,
            Err(e) => {
                interface.print(&e.to_string());
                interface.window.get_input();
                break;
            }
        }        
    }
}
// #[actix_rt::main]
// async fn main() -> std::io::Result<()> {
//     simple_logger::init_with_level(log::Level::Debug).unwrap();

//     let mut listenfd = ListenFd::from_env();
//     let mut server = HttpServer::new(|| {
//         App::new()
//             .service(web::scope("/session")
//                 .route("/new", web::post().to(new_session))
//                 .route("", web::get().to(get_session)))
//             .service(web::scope("/story")
//                 .route("/{name}/new", web::post().to(new_story)))
//             .service(web::scope("/memory/{name}")
//                 .service(web::scope("/byte")
//                     .route("/{address}", web::get().to(read_byte))
//                     .route("/{address}/{value}", web::put().to(write_byte)))
//                 .service(web::scope("/word")
//                     .route("/{address}", web::get().to(read_word))
//                     .route("/{address}/{value}", web::put().to(write_word))))
//             .service(web::scope("/text/{name}")
//                 .route("/{address}/decode", web::get().to(read_text))
//                 .route("/encode/{string}", web::get().to(encode_text)))
//             .route("/object/{name}/tree/{end}", web::get().to(object_tree))
//             .service(web::scope("/object/{name}/{number}")
//                 .route("", web::get().to(get_object))
//                 .route("", web::delete().to(remove_object))
//                 .route("/{parent}", web::put().to(insert_object))
//                 .route("/attribute/{attribute}", web::get().to(has_object_attribute))
//                 .route("/attribute/{attribute}", web::put().to(set_object_attribute))
//                 .route("/attribute/{attribute}", web::delete().to(clear_object_attribute)) 
//                 .route("/property/{property}", web::get().to(get_object_property))
//                 .route("/property/{property}/{value}", web::put().to(put_object_property)))
//             .service(web::scope("/instruction/{name}/{address}")
//                 // .route("/decode", web::get().to(instruction))
//                 // .route("/execute", web::get().to(execute_instruction))
//                 .route("/run", web::get().to(run)))
//             .route("routine/{name}/{address}/decode", web::get().to(get_routine))
//             .wrap(middleware::Performance)

//     });


//     server = if let Some(l) = listenfd.take_tcp_listener(0).unwrap() {
//         server.listen(l)?
//     } else {
//         server.bind("127.0.0.1:3000")?
//     };

//     server.run().await
// }
