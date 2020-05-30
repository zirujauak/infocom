use std::convert::TryFrom;

use super::InfocomError;

#[derive(Clone, Copy, Debug)]
pub enum Version {
    V(u8)
}

impl Default for Version {
    fn default() -> Self { Version::V(0) }
}

pub trait ZValue {
    fn new(values: &[u8]) -> Self;
    fn size() -> usize;
}

pub struct ZByte {
    pub value: u8
}

impl ZValue for ZByte {
    fn new(values: &[u8]) -> ZByte {
        ZByte { value: values[values.len() - 1] }
    }

    fn size() -> usize {
        1
    }
}

pub struct ZWord {
    pub value: u16
}

impl ZValue for ZWord {
    fn new(values: &[u8]) -> ZWord {
        let high_byte = values[values.len() - 2];
        let low_byte = values[values.len() - 1];
        let value:u16 = (((high_byte as u16) << 8) & 0xFF00) | ((low_byte as u16) & 0xFF);
        ZWord { value }
    }

    fn size() -> usize {
        2
    }
}

pub struct MemoryMap {
    pub version: Version,
    memory_map: Vec<u8>,
    dynamic_restore: Vec<u8>,
    static_mark: usize,
}

impl TryFrom<Vec<u8>> for MemoryMap {
    type Error = InfocomError;

    fn try_from(data: Vec<u8>) -> Result<MemoryMap, Self::Error> {
        if data.len() > 0 {
            let high:u16 = data[0xe].into();
            let low:u16 = data[0xf].into();
            let mark:usize = (((high << 8) & 0xFF00) | (low & 0xFF)).into();  
            let dynamic_restore = data[0..mark].to_vec();  
            Ok(MemoryMap { version: Version::V(data[0]),
                           memory_map: data,
                           dynamic_restore,
                           static_mark: mark})
        } else {
            Err(InfocomError::Memory(format!("Invalid memory map data")))
        }
    }
}

impl MemoryMap {
    fn len(&self) -> usize {
        self.memory_map.len()
    }

    /// Gets a (read-only) copy of the memory map
    /// 
    /// # Examples
    /// 
    /// ```
    /// use memory::Memory;
    /// 
    /// let memory_map = mem.get_memory();
    /// ```
    pub fn get_memory(&self) -> Vec<u8> {
        self.memory_map.to_vec()
    }
    
    pub fn get_dynamic_restore(&self) -> Vec<u8> {
        self.dynamic_restore.to_vec()
    }

    /// Read a byte from the memory map, restricted to the bottom 64k of memory.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use memory::Memory;
    /// 
    /// let b = mem.get_byte(0x12).unwrap();
    /// ```
    pub fn get_byte(&self, address: usize) -> Result<u8, InfocomError> {
        if address <= 0xFFFF && address < self.len() {
            Ok(self.memory_map[address])
        } else {
            Err(InfocomError::ReadViolation(address, self.len()))
        }
    }

    /// Read a word from the memory map, restricted to the bottom 64k of memory.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use memory::Memory;
    /// 
    /// let w = mem.get_word(0x12).unwrap();
    /// ```
    pub fn get_word(&self, address: usize) -> Result<u16, InfocomError> {
        let high = self.get_byte(address)?;
        let low = self.get_byte(address + 1)?;
        Ok((((high as u16) << 8) & 0xFF00) | ((low as u16) & 0xFF))
    }

    /// Write a byte to the dynamic region of memory.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use memory::Memory;
    /// 
    /// mem.set_byte(0x12, 0xFF)?;
    /// ```
    pub fn set_byte(&mut self, address: usize, value: u8) -> Result<(), InfocomError> {
        let mark = self.static_mark;
        if address < mark {
            self.memory_map[address] = value;
            Ok(())
        } else {
            Err(InfocomError::WriteViolation(address, mark - 1))
        }
    }

    /// Write a word to the dynamic region of memory.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use memory::Memory;
    /// 
    /// mem.set_word(0x12, 0xFFFF)?;
    /// ```
    pub fn set_word(&mut self, address: usize, value: u16) -> Result<(), InfocomError> {
        self.set_byte(address, (value >> 8) as u8 & 0xFF)?;
        self.set_byte(address + 1, value as u8 & 0xFF)
    }
}
