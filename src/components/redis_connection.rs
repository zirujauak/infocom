use std::collections::HashMap;

use log::{debug,error,warn};
use redis::{Client, Connection, RedisError, RedisResult, Value};

struct RedisTransaction {
    connection: Connection,
    expectations: Vec<Value>
}

pub struct RedisConnection {
    client: Client,
    connection: Connection,
    transactions: HashMap<String,RedisTransaction>,
}

impl RedisConnection {
    pub fn new(url: &str) -> Result<RedisConnection, RedisError> {
        let client = Client::open(url)?;
        let connection = client.get_connection()?;
        Ok(RedisConnection { client, connection, transactions: HashMap::new() })
    }

    pub fn open_transaction(&mut self, key: &str) -> RedisResult<Value> {
        if self.transactions.contains_key(key) {
            warn!("Transaction already opened for {}", key);
            return Ok(Value::Okay)
        } else {
            let con = self.client.get_connection()?;
            self.transactions.insert(String::from(key), RedisTransaction { connection: con, expectations: Vec::new() });
            let txn = self.transactions.get_mut(key).unwrap();
            redis::cmd("WATCH").arg(key).query(&mut txn.connection)?;
            redis::cmd("MULTI").query(&mut txn.connection)
        }
    }

    pub fn commit_transaction(&mut self, key: &str) -> RedisResult<Value> {
        if let Some(mut txn) = self.transactions.remove(key) {
            match redis::cmd("EXEC").query(&mut txn.connection) {
                Ok(v) => {
                    match &v {
                        Value::Bulk(results) => {
                            for (i, expect) in txn.expectations.iter().enumerate() {
                                if results[i] != *expect {
                                    error!("Transaction element failed: {:?} => {:?}", txn.expectations, results);
                                    return Err(RedisError::from((redis::ErrorKind::ClientError, "Transaction failure", format!("Expectations not met for {}: {:?} => {:?}", key, txn.expectations, results)))) 
                                }
                            }

                            Ok(v)
                        }
                        _ => Err(RedisError::from((redis::ErrorKind::ClientError, "Transaction failure", format!("Expected Value::Bulk, got {:?}", v))))
                    }
                },
                Err(e) => {
                    error!("Error committing transaction: {:?}", e);
                    Err(e)
                }
            }
        } else {
            Err(RedisError::from((redis::ErrorKind::ClientError, "No transcation", format!("No open transaction for key {}", key))))
        }
    }

    pub fn get<T>(&mut self, key: &str) -> RedisResult<T> 
    where 
        T: redis::FromRedisValue 
    {
      redis::cmd("GET").arg(key).query(&mut self.connection)
    }

    pub fn set<T>(&mut self, txn_key: &str, key: &str, value: T) -> RedisResult<Value> 
    where
        T: redis::ToRedisArgs
    {
        if let Some(txn) = self.transactions.get_mut(txn_key) {
            txn.expectations.push(Value::Okay);
            redis::cmd("SET")
                .arg(key).arg(value)
                .arg("EX").arg(3600)
                .query(&mut txn.connection)
        } else {
            Err(RedisError::from((redis::ErrorKind::ClientError, "No transcation", format!("No open transaction for key {}", txn_key))))
        }
    }

    pub fn set_new<T>(&mut self, txn_key: &str, key: &str, value: T) -> RedisResult<Value> 
    where
        T: redis::ToRedisArgs
    {
        if let Some(txn) = self.transactions.get_mut(txn_key) {
            txn.expectations.push(Value::Okay);
            redis::cmd("SET")
                .arg(key).arg(value)
                .arg("EX").arg(3600)
                .arg("NX")
                .query(&mut txn.connection)
        } else {
            Err(RedisError::from((redis::ErrorKind::ClientError, "No transcation", format!("No open transaction for key {}", txn_key))))
        }
    }

    pub fn set_replace<T>(&mut self, txn_key: &str, key: &str, value: T) -> RedisResult<Value> 
    where
        T: redis::ToRedisArgs
    {
        if let Some(txn) = self.transactions.get_mut(txn_key) {
            txn.expectations.push(Value::Okay);
            redis::cmd("SET")
                .arg(key).arg(value)
                .arg("EX").arg(3600)
                .arg("XX")
                .query(&mut txn.connection)
        } else {
            Err(RedisError::from((redis::ErrorKind::ClientError, "No transcation", format!("No open transaction for key {}", txn_key))))
        }
    }

    pub fn touch(&mut self, key: &str) -> RedisResult<Value> {
        redis::cmd("EXPIRE")
            .arg(key).arg(3600)
            .query(&mut self.connection)
    }
}
