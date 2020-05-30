use log::{debug, error, warn};
use serde::{Deserialize, Serialize};

use super::InfocomError;
use super::memory::{MemoryMap, Version};
use super::state::FrameStack;
use super::text::Decoder;

#[derive(Serialize, Deserialize, Clone)]
struct Property {
    number: usize,
    address: usize,
    size: u16,
    data_address: usize,
    data: Vec<u8>,
}

impl Property {
    fn load(mem: &MemoryMap, prop_addr: usize) -> Result<Option<Property>, InfocomError> {
        let size_byte = mem.get_byte(prop_addr)?;
        if size_byte == 0 {
            return Ok(None);
        }

        let (number, size, skip) = match mem.version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                let size = (size_byte as u16 / 32) + 1;
                let number = size_byte as usize & 0x1F;
                (number, size, 1)
            },
            Version::V(4) | Version::V(5) | Version::V(6) | Version::V(7) | Version::V(8) => {
                if size_byte & 0x80 == 0x80 {
                    let size_byte_2 = mem.get_byte(prop_addr + 1)? as u16;
                    let size:u16 = if size_byte_2 & 0x3F == 0 {
                        64
                    } else {
                        size_byte_2 & 0x3F
                    };
                    let number = size_byte as usize & 0x3F;
                    (number, size, 2)
                } else {
                    let size = match size_byte & 0x40 {
                        0x40 => 2,
                        _ => 1
                    };
                    let number = size_byte as usize & 0x3F;
                    (number, size, 1)
                }
            },
            _ => return Err(InfocomError::Version(mem.version))
        };

        let mut data:Vec<u8> = Vec::new();
        for i in 0..size {
            data.push(mem.get_byte(prop_addr + skip + i as usize)?);
        }

        Ok(Some(Property { number,
                           address: prop_addr,
                           size,
                           data_address: prop_addr + skip,
                           data }))
    }

    fn save(&self, state: &mut FrameStack) -> Result<(), InfocomError> {
        for (i, d) in self.data.iter().enumerate() {
            state.set_byte(self.data_address + i, *d)?;
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct PropertyTable {
    address: usize,
    short_name: String,
    properties: Vec<Property>,
}

#[derive(Serialize, Deserialize)]
pub struct Object {
    number: usize,
    address: usize,
    attribute_count: usize,
    attributes: u64,
    parent: u16,
    sibling: u16,
    child: u16,
    property_table: PropertyTable
}

pub struct ObjectTable {
    address: usize,
    default_properties: Vec<u16>,
}

impl PropertyTable {
    fn load(mem: &MemoryMap, address: usize) -> Result<PropertyTable, InfocomError> {
        let short_name_size = mem.get_byte(address)? as usize;
        let decoder = Decoder::new(mem)?;
        let short_name = if short_name_size > 0 {
            decoder.decode(address + 1)?
        } else {
            String::new()
        };
        let mut properties: Vec<Property> = Vec::new();
        let mut prop_addr = address + 1 + (short_name_size * 2);

        loop {
            match Property::load(mem, prop_addr)? {
                Some(p) => {
                    prop_addr = p.data_address + p.size as usize;
                    properties.push(p);
                }
                None => { break; }
            }
        };

        Ok(PropertyTable { address,
                           short_name,
                           properties })
    }

    fn get_property(&self, property_number: usize) -> Option<&Property> {
        for p in self.properties.iter() {
            if p.number == property_number {
                return Some(p);
            }
        }

        None
    }

    fn set_property(&mut self, property_number: usize, value: u16) -> Result<(), InfocomError> {
        if let Some(p) = self.get_property(property_number) {
            if p.size < 3 {
                // Rebuild the property table, replacing the updated Property data
                let mut new_t:Vec<Property> = Vec::new();
                for o_p in self.properties.iter() {
                    if o_p.number != property_number {
                        new_t.push(Property { data: Vec::from(o_p.data.clone()), .. *o_p});
                    } else {
                        new_t.push(Property { data: if p.size == 1 {
                            vec![value as u8 & 0xFF]
                        } else {
                            vec![((value >> 8) as u8 & 0xFF), value as u8 & 0xFF]
                        }, .. *p });
                    }
                }
                self.properties = new_t;
                Ok(())
            } else {
                Err(InfocomError::Memory(format!("Write to property ${:02x} with length greater than 2", property_number)))
            }
        } else {
            Err(InfocomError::Memory(format!("Write to property ${:02x} that does not exist", property_number)))
        }
    }

    fn save_property(&self, state: &mut FrameStack, property_number: usize) -> Result<(), InfocomError> {
        for p in &self.properties {
            if p.number == property_number {
                return p.save(state)
            }
        }

        Err(InfocomError::Memory(format!("Save of unowned property: ${:02}", property_number)))
    }
}

impl Object {
    fn load(mem: &MemoryMap, number: usize, address: usize) -> Result<Object, InfocomError> {
        match mem.version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                let attr_1 = mem.get_word(address)?;
                let attr_2 = mem.get_word(address + 2)?;
                let attributes:u64 = (((attr_1 as u64) << 16) & 0xFFFF0000) | ((attr_2 as u64) & 0xFFFF);
                let parent = mem.get_byte(address + 4)? as u16;
                let sibling = mem.get_byte(address + 5)? as u16;
                let child = mem.get_byte(address + 6)? as u16;
                let prop_addr = mem.get_word(address + 7)? as usize;
                let property_table = PropertyTable::load(mem, prop_addr)?;
                Ok(Object{ number,
                           address,
                           attribute_count: 32,
                           attributes, 
                           parent,
                           sibling,
                           child,
                           property_table})
            },
            _ => {
                let attr_1 = mem.get_word(address)?;
                let attr_2 = mem.get_word(address + 2)?;
                let attr_3 = mem.get_word(address + 4)?;
                let attributes:u64 = (((attr_1 as u64) << 32) & 0xFFFF00000000) | (((attr_2 as u64) << 16)& 0xFFFF0000) | (attr_3 as u64) & 0xFFFF;
                let parent = mem.get_word(address + 6)? as u16;
                let sibling = mem.get_word(address + 8)? as u16;
                let child = mem.get_word(address + 10)? as u16;
                let prop_addr = mem.get_word(address + 12)? as usize;
                let property_table = PropertyTable::load(mem, prop_addr)?;
                Ok(Object{ number,
                           address,
                           attribute_count: 48,
                           attributes, 
                           parent,
                           sibling,
                           child,
                           property_table})
            }
        }
    }

    pub fn save_family(&self, state: &mut FrameStack) -> Result<(), InfocomError>
    {
        match state.get_memory().version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                state.set_byte(self.address + 4, self.parent as u8)?;
                state.set_byte(self.address + 5, self.sibling as u8)?;
                state.set_byte(self.address + 6, self.child as u8)?;
            },
            _ => {
                state.set_word(self.address + 6, self.parent)?;
                state.set_word(self.address + 8, self.sibling)?;
                state.set_word(self.address + 10, self.child)?;
            }
        }

        Ok(())
    }

    pub fn save_attributes(&self, state: &mut FrameStack) -> Result<(), InfocomError> {
        match state.get_memory().version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                let attr_1:u16 = ((self.attributes >> 16) & 0xFFFF) as u16;
                let attr_2:u16 = (self.attributes & 0xFFFF) as u16;
                state.set_word(self.address, attr_1)?;
                state.set_word(self.address + 2, attr_2)?;
            },
            _ => {
                let attr_1 = ((self.attributes >> 32) & 0xFFFF) as u16;
                let attr_2 = ((self.attributes >> 16) & 0xFFFF) as u16;
                let attr_3 = (self.attributes & 0xFFFF) as u16;
                state.set_word(self.address, attr_1)?;
                state.set_word(self.address + 2, attr_2)?;
                state.set_word(self.address + 4, attr_3)?;
            }
        }

        Ok(())
    }

    pub fn save_property(&self, state: &mut FrameStack, property_number: usize) -> Result<(), InfocomError> {
        self.property_table.save_property(state, property_number)
    }

    pub fn get_short_name(&self) -> String {
        self.property_table.short_name.clone()
    }

    pub fn get_parent(&self) -> u16 {
        self.parent
    }

    pub fn get_sibling(&self) -> u16 {
        self.sibling
    }

    pub fn get_child(&self) -> u16 {
        self.child
    }

    pub fn has_attribute(&self, attribute: usize) -> Result<bool, InfocomError> {
        if attribute <= self.attribute_count {
            Ok(self.attributes >> (self.attribute_count - attribute - 1) & 0x1 == 0x1)
        } else {
            Err(InfocomError::Memory(format!("Invalid attribute ${:02x}", attribute)))
        }
    }

    pub fn set_attribute(&mut self, attribute: usize) -> Result<u64, InfocomError> {
        if attribute <= self.attribute_count {
            let mask:u64 = 1 << (self.attribute_count - attribute - 1);
            let attributes = self.attributes | mask;
            self.attributes = attributes;
            Ok(attributes)
        } else {
            warn!("Attempt to set an invalid attribute: ${:02x}", attribute);
            Ok(self.attributes)
        }
    }

    pub fn clear_attribute(&mut self, attribute: usize) -> Result<u64, InfocomError> {
        if attribute <= self.attribute_count {
            let mut mask:u64 = 0;
            for _ in 0..(self.attribute_count / 8) {
                mask = mask << 8 | 0xFF;
            }
            let bit:u64 = 1 << (self.attribute_count - attribute - 1);
            mask ^= bit;
            let attributes = self.attributes & mask;
            self.attributes = attributes;
            Ok(attributes)
        } else {
            warn!("Attempt to set an invalid attribute: ${:02x}", attribute);
            Ok(self.attributes)
        }
    }

    fn get_property(&self, property: usize) -> Option<&Property> {
        self.property_table.get_property(property)
    }
    
    fn set_property(&mut self, property_number: usize, value: u16) -> Result<(), InfocomError> {
        self.property_table.set_property(property_number, value)
    }

    fn next_property_number(&self, property: usize) -> Result<u8, InfocomError> {
        let mut i = self.property_table.properties.iter();

        if property == 0 {
            if let Some(r) = i.next() {
                return Ok(r.number as u8);
            } else {
                return Ok(0);
            }
        }
        while let Some(p) = i.next() {
            if p.number == property {
                if let Some(r) = i.next() {
                    return Ok(r.number as u8);
                } else {
                    return Ok(0);
                }
            }
        }

        Err(InfocomError::Memory(format!("Next property for object ${:04x} after ${:02x} - starting property not found", self.number, property)))
    }
}

impl ObjectTable {
    pub fn new(mem: &MemoryMap) -> Result<ObjectTable, InfocomError> {
        let address = mem.get_word(0x0a)? as usize;
        let mut default_properties:Vec<u16> = Vec::new();

        match mem.version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                for i in 0..31 {
                    let v = mem.get_word(address + (2 * i))?;
                    default_properties.push(v);
                }
            },
            Version::V(4) | Version::V(5) | Version::V(6) | Version::V(7) | Version::V(8) => {
                for i in 0..63 {
                    let v = mem.get_word(address + (2 * i))?;
                    default_properties.push(v);
                }
            },
            _ => return Err(InfocomError::Version(mem.version))
        }

        debug!("${:04x}, default properties: {:?}", address, default_properties);
        Ok(ObjectTable { address,
                         default_properties })
    }

    pub fn get_object(&self, memory: &MemoryMap, object_number: usize) -> Result<Object, InfocomError> {
        let object_address = match memory.version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                self.address + 62 + ((object_number - 1) * 9)
            },
            _ => self.address + 126 + ((object_number - 1) * 14)
        };

        let o = Object::load(memory, object_number, object_address)?;
        Ok(o)
    }

    pub fn remove_object(&mut self, state: &mut FrameStack, object_number: usize) -> Result<Object, InfocomError> {
        let mut o = self.get_object(state.get_memory(), object_number)?;
        debug!("remove object: {}, having sibling {}, from {}", object_number, o.sibling, o.parent);
        if o.parent != 0 {
            let mut p = self.get_object(state.get_memory(), o.parent as usize)?;
            if p.child == object_number as u16 {
                // Object is first child of parent, replace parent's child with this object's sibling
                debug!("first child");
                p.child = o.sibling;
                p.save_family(state)?;
            } else {
                // Object is a sibling, replace sibling reference
                let mut prev = self.get_object(state.get_memory(), p.child as usize)?;
                while prev.sibling != object_number as u16 {
                    prev = self.get_object(state.get_memory(), prev.sibling as usize)?;
                }
                debug!("Sibling of {}", prev.number);
                prev.sibling = o.sibling;
                prev.save_family(state)?;
            }

            o.parent = 0;
            o.sibling = 0;
            debug!("save object");
            o.save_family(state)?;
        }

        Ok(o)
    }

    pub fn insert_object(&mut self, state: &mut FrameStack, object_number: usize, new_parent: usize) -> Result<Object, InfocomError> {
        let mut o = self.remove_object(state, object_number)?;
        let mut p = self.get_object(state.get_memory(), new_parent)?;
        debug!("insert object {} into {}, having child {}", o.number, p.parent, p.sibling);
        o.sibling = p.child;
        o.parent = new_parent as u16;
        p.child = object_number as u16;
        debug!("save object");
        o.save_family(state)?;
        debug!("save parent");
        p.save_family(state)?;

        Ok(o)
    }

    pub fn has_attribute(&self, memory: &MemoryMap, object_number: usize, attribute_number: usize) -> Result<bool, InfocomError> {
        let o = self.get_object(memory, object_number)?;
        o.has_attribute(attribute_number)
    }

    pub fn set_attribute(&mut self, state: &mut FrameStack, object_number: usize, attribute_number: usize) -> Result<Object, InfocomError> {
        let mut o = self.get_object(state.get_memory(), object_number)?;
        o.set_attribute(attribute_number)?;
        o.save_attributes(state)?;
        Ok(o)
    }
    
    pub fn clear_attribute(&mut self, state: &mut FrameStack, object_number: usize, attribute_number: usize) -> Result<Object, InfocomError> {
        let mut o = self.get_object(state.get_memory(), object_number)?;
        o.clear_attribute(attribute_number)?;
        o.save_attributes(state)?;
        Ok(o)
    }

    pub fn read_property_data(&self, memory: &MemoryMap, object_number: usize, property_number: usize) -> Result<Vec<u8>, InfocomError> {
        let o = self.get_object(memory, object_number)?;
        if let Some(p) = o.get_property(property_number) {
            Ok(p.data.to_vec())
        } else {
            if let Some(v) = self.default_properties.get(property_number - 1) {
                let b1 = ((v >> 8) & 0xFF) as u8;
                let b2 = (v & 0xFF) as u8;
                Ok(vec![b1, b2])
            } else {
                Err(InfocomError::Memory(format!("Invalid property number: ${:02x}", property_number)))
            }
        }
    }

    fn get_default_property(&self, property_number: usize) -> Result<Vec<u8>, InfocomError> {
        if let Some(v) = self.default_properties.get(property_number - 1) {
            let b1 = ((v >> 8) & 0xFF) as u8;
            let b2 = (v & 0xFF) as u8;
            Ok(vec![b1, b2])
        } else {
            Err(InfocomError::Memory(format!("Invalid property number: ${:02x}", property_number)))
        }
    }

    pub fn get_property_value(&self, memory: &MemoryMap, object_number: usize, property_number: usize) -> Result<u16, InfocomError> {
        match self.get_object(memory, object_number)?.get_property(property_number) {
            Some(p) => if p.size == 1 {
                Ok(p.data[0] as u16)
            } else if p.size == 2 {
                Ok(((p.data[0] as u16) << 8) & 0xFF00 | (p.data[1] as u16 & 0xFF))
            } else {
                Err(InfocomError::Memory(format!("Attempt to read property ${:02x} data on object ${:04x} with length ${:02x}", property_number, object_number, p.size)))
            },            
            None => {
                if let Some(v) = self.default_properties.get(property_number - 1) {
                    debug!("Read default property {:02x}: ${:04x}", property_number, v);
                    Ok(*v)
                } else {
                    Err(InfocomError::Memory(format!("Invalid property number: ${:02x}", property_number)))
                }
            }
        }
    }

    pub fn put_property_data(&mut self, state: &mut FrameStack, object_number: usize, property_number: usize, value: u16) -> Result<Object, InfocomError> {
        let mut o = self.get_object(state.get_memory(), object_number)?;
        match o.get_property(property_number) {
            Some(p) => {
                if p.size > 2 {
                    return Err(InfocomError::Memory(format!("Attempt to write property ${:02x} on object ${:04x} with length ${:02x}", property_number, object_number, p.size)))
                }

                o.set_property(property_number, value)?;
                o.save_property(state, property_number)?;
                Ok(o)
            },
            None => Err(InfocomError::Memory(format!("Set property ${:02x} on object ${:04x} that doesn't have the specified property", property_number, object_number)))
        } 
    }

    pub fn get_next_property(&self, memory: &MemoryMap, object_number: usize, property_number: usize) -> Result<u8, InfocomError> {
        self.get_object(memory, object_number)?.next_property_number(property_number)
    }

    pub fn get_property_address(&self, memory: &MemoryMap, object_number: usize, property_number: usize) -> Result<usize, InfocomError> {
        match self.get_object(memory, object_number)?.get_property(property_number) {
            Some(p) => Ok(p.data_address),
            None => Ok(0)
        } 
    }

    pub fn get_property_len(&self, memory: &MemoryMap, property_address: usize) -> Result<usize, InfocomError> {
        let b = memory.get_byte(property_address - 1)?;
        match memory.version {
            Version::V(1) | Version::V(2) | Version::V(3) => {
                Ok(((b as usize / 32) & 0x7) + 1)
            },
            _ => {
                if b & 0x80 == 0x80 {
                    let l = b as usize & 0x3F;
                    if l == 0 {
                        Ok(64)
                    } else {
                        Ok(l)
                    }
                } else {
                    if b & 0x40 == 0x40 {
                        Ok(2)
                    } else {
                        Ok(1)
                    }
                }
            }
        }
    }
}
