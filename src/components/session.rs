use std::convert::TryFrom;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use log::{debug, error};
use uuid::Uuid;
use redis::{FromRedisValue, RedisResult, ToRedisArgs, Value};

use super::memory;
use super::redis_connection::RedisConnection;
use super::InfocomError;

#[derive(Debug, Deserialize, Serialize)]
pub struct Session {
    pub id: String,
    stories: HashMap<String, String>
}

impl FromRedisValue for Session {
    fn from_redis_value(v: &Value) -> RedisResult<Session> {
        match *v {
            Value::Data(ref bytes) => Ok(serde_json::from_str(&String::from_utf8(bytes.to_vec()).unwrap()).unwrap()),
            _ => Err(redis::RedisError::from((redis::ErrorKind::TypeError, "Response was of incompatible type", format!("{:?} (response was {:?})", "response not Session compatible", v))))       
        }
    }
}

impl ToRedisArgs for &Session {
    fn write_redis_args<W>(&self, out: &mut W) 
    where
        W: ?Sized + redis::RedisWrite
    {
        let bytes = serde_json::to_string(self).unwrap();
        out.write_arg(bytes.as_bytes())
    }
}

impl ToRedisArgs for &&mut Session {
    fn write_redis_args<W>(&self, out: &mut W) 
    where
        W: ?Sized + redis::RedisWrite
    {
        let bytes = serde_json::to_string(self).unwrap();
        out.write_arg(bytes.as_bytes())
    }
}

impl TryFrom<&str> for Session {
    type Error = InfocomError;

    fn try_from(id: &str) -> Result<Session, InfocomError> {
        let mut con = RedisConnection::new("redis://localhost")?;
        let session:Session = con.get(id)?;
        con.touch(id)?;
        Ok(session)
    }
}

impl Session {
    pub fn new() -> Result<Session, InfocomError> {
        let id = Uuid::new_v4().to_string();
        let stories = HashMap::new();
        let session = Session { id: String::from(&id), stories };
        let mut con = RedisConnection::new("redis://localhost")?;
        con.open_transaction(&id)?;
        con.set_new(&id, &id, &session)?;
        con.commit_transaction(&id)?;
        Ok(session)
    }

    pub fn add_story(&mut self, name: String, mem: memory::MemoryMap) -> Result<(), InfocomError> {
        if self.stories.contains_key(&name) {
            error!("Story '{}' already exists.", name);
            Err(InfocomError::Session(format!("Story '{}' already exists.", name)))
        } else {
            let id = Uuid::new_v4().to_string();
            self.stories.insert(name, String::from(&id));
            let mut con = RedisConnection::new("redis://localhost")?;
            con.open_transaction(&self.id)?;
            con.set_new(&self.id, &id, &mem)?;
            con.set_replace(&self.id, &self.id, &self)?;
            con.commit_transaction(&self.id)?;
            Ok(())
        }
    }

    pub fn load(&mut self, name: &str) -> Result<memory::MemoryMap, InfocomError> {
        let id = self.stories.get(name).unwrap();
        memory::MemoryMap::try_from(id)
    }

    pub fn save(&mut self, name: &str, mem: memory::MemoryMap) -> Result<(), InfocomError> {
        let id = self.stories.get(name).unwrap();
        let mut con = RedisConnection::new("redis://localhost")?;
        con.open_transaction(&id)?;
        con.set_replace(&id, &id, &mem)?;
        con.commit_transaction(&id)?;
        Ok(())
    }
}