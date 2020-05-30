use std::error;
use std::fmt;
use redis::RedisError;

mod redis_connection;

pub mod memory;
pub mod session;
pub mod text;
pub mod object_table;
pub mod state;
pub mod instruction;
pub mod interface;
pub mod dictionary;

#[derive(Debug)]
pub enum InfocomError {
    Memory(String),
    ReadViolation(usize, usize),
    WriteViolation(usize, usize),
    Text(String),
    API(String),
    Session(String),
    Version(memory::Version),
    Redis(RedisError)
}

impl fmt::Display for InfocomError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            InfocomError::ReadViolation(ref a, ref b) => f.write_fmt(format_args!("Invalid read from ${:06x} beyond end of static memory ${:06x}", a, std::cmp::min(0xFFFF, *b))),
            InfocomError::WriteViolation(ref a, ref b) => f.write_fmt(format_args!("Invalid write to ${:06x} beyond end of dynamic memory ${:06x}", a, b)),
            InfocomError::Version(ref e) => f.write_fmt(format_args!("Unsupported Z-Machine version: {:?}", e)),
            InfocomError::Redis(ref e) => e.fmt(f),
            InfocomError::Memory(ref e) => e.fmt(f),
            InfocomError::Text(ref e) => e.fmt(f),
            InfocomError::API(ref e) => e.fmt(f),
            InfocomError::Session(ref e) => e.fmt(f)
        }
    }
}

impl error::Error for InfocomError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            InfocomError::Redis(ref e) => Some(e),
            _ => None
        }
    }
}

impl From<RedisError> for InfocomError {
    fn from(err: RedisError) -> InfocomError {
        InfocomError::Redis(err)
    }
}