extern crate log;
extern crate simple_logger;
extern crate rand;

use std::convert::TryFrom;

mod components;

use components::memory::MemoryMap;
use components::state::FrameStack;
use components::instruction;
use components::interface::{ Curses, Interface };


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
