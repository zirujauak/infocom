use super::memory::{ MemoryMap, Version };
use super::InfocomError;
use super::dictionary::Dictionary;

use log::debug;
use rand::rngs::ThreadRng;
use rand::Rng;

#[derive(Clone, Debug)]
pub struct Routine {
    address: usize,
    default_variables: Vec<u16>,
    instruction_address: usize,
}

impl Routine {
    pub fn new(mem: &MemoryMap, address: usize) -> Result<Routine, InfocomError> {
        let variable_count = read_byte(mem, address) as usize;
        let mut default_variables:Vec<u16> = vec![0; variable_count];

        let instruction_address = match mem.version {
            Version::V(1) | Version::V(2) | Version::V(3) | Version::V(4) => {
                for i in 0..variable_count {
                    default_variables[i] = read_word(mem, address + 1 + (i * 2));
                }

                address + 1 + (2 * variable_count)
            },
            _ => address + 1
        };

        Ok(Routine { address, default_variables, instruction_address })
    }
}

#[derive(Clone, Debug)]
pub struct Frame {
    routine: Routine,
    local_variables: Vec<u16>,
    stack: Vec<u16>,
    pc: usize,
    return_variable: Option<u8>,
    return_address: usize,
}


fn read_byte(mem: &MemoryMap, address: usize) -> u8 {
    mem.get_memory()[address]
}

fn read_word(mem: &MemoryMap, address: usize) -> u16 {
    let high = mem.get_memory()[address];
    let low = mem.get_memory()[address + 1];

    (((high as u16) << 8) & 0xFF00) | low as u16 & 0xFF
}

impl Frame {
    pub fn new(routine: Routine, arguments: Vec<u16>, return_variable: Option<u8>, return_address: usize) -> Result<Frame, InfocomError> {
        let mut local_variables:Vec<u16> = routine.default_variables.clone();

        for (i, arg) in arguments.iter().enumerate() {
            local_variables[i] = *arg;
        }

        let pc = routine.instruction_address;

        debug!("Frame: ${:06x} {:?}, @ ${:06x}, S->{:?}, ret @ ${:06x}", routine.address, local_variables, routine.instruction_address, return_variable, return_address);
        Ok(Frame { routine, local_variables, stack: Vec::new(), pc, return_variable, return_address })
    }

    pub fn push(&mut self, value: u16) {  
        self.stack.push(value);
    }

    pub fn peek(&self) -> Result<u16, InfocomError> {
        match self.stack.last() {
            Some(v) => Ok(*v),
            None => Err(InfocomError::Memory(format!("Peek into empty stack")))
        }
    }

    pub fn pop(&mut self) -> Result<u16, InfocomError> {
        if self.stack.len() > 0 {
            Ok(self.stack.remove(self.stack.len() - 1))
        } else {
            Err(InfocomError::Memory(format!("Pop from empty stack")))
        }
    }
}

pub struct FrameStack<'a> {
    memory: &'a mut MemoryMap,
    global_variable_table_address: usize,
    stack: Vec<Frame>,
    pub current_frame: Frame,
    rng: ThreadRng,
    pub dictionary: Dictionary,
}

impl<'a> FrameStack<'a> {
    pub fn new(mem: &'a mut MemoryMap) -> Result<FrameStack, InfocomError> {
        let pc = mem.get_word(0x06)? as usize;
        let global_variable_table_address = mem.get_word(0x0C)? as usize;
        let r = Routine { address: pc, default_variables: Vec::new(), instruction_address: pc };
        let f = Frame::new(r, Vec::new(), None, 0)?;
        let stack = Vec::new();
        let rng = rand::thread_rng();
        let dictionary = Dictionary::new(&mem)?;
        //debug!("dictionary: {:?}", dictionary);

        Ok(FrameStack { memory: mem, global_variable_table_address, stack, current_frame: f, rng, dictionary })
    }

    // pub fn analyze_text(&mut self, text: &String, parse_table_address: usize) -> Result<(),InfocomError> {
    //     self.dictionary.analyze_text(self, text, parse_table_address)
    // }

    pub fn pc(&self) -> usize {
        self.current_frame.pc
    }

    pub fn random(&mut self, range: u16) -> Result<u16,InfocomError> {
        // TODO: Handle "predictable mode"
        Ok(self.rng.gen_range(0, range) as u16 + 1)
    }

    pub fn get_memory(&self) -> &MemoryMap {
        self.memory
    }

    pub fn set_byte(&mut self, address: usize, value: u8) -> Result<(),InfocomError> {
        debug!("Write ${:02x} to ${:04x}", value, address);
        self.memory.set_byte(address, value)
    }

    pub fn set_word(&mut self, address: usize, value: u16) -> Result<(),InfocomError> {
        debug!("Write ${:04x} to ${:04x}", value, address);
        self.memory.set_word(address, value)
    }

    pub fn unpack_address(&self, packed_address: u16) -> Result<usize,InfocomError> {
        match self.memory.version {
            Version::V(1) | Version::V(2) | Version::V(3) => Ok(packed_address as usize * 2),
            Version::V(4) | Version::V(5) => Ok(packed_address as usize * 4),
            Version::V(8) => Ok(packed_address as usize * 8),
            _ => return Err(InfocomError::Memory(format!("Unimplemented version: {:?}", self.memory.version)))
        }
    }

    pub fn call(&mut self, packed_address: u16, arguments: Vec<u16>, return_variable: Option<u8>, return_address: usize) -> Result<usize, InfocomError> {
        if packed_address == 0 {
            if let Some(v) = return_variable {
                self.set_variable(v, 0, false)?;
            }

            Ok(return_address)
        } else {
            let address = self.unpack_address(packed_address)?;
            let routine = Routine::new(self.memory, address)?;
            self.stack.push(self.current_frame.clone());
            self.current_frame = Frame::new(routine, arguments, return_variable, return_address)?;
            Ok(self.current_frame.pc)
        }
    }

    pub fn return_from(&mut self, return_value: u16) -> Result<usize, InfocomError> {
        let return_variable = self.current_frame.return_variable;
        debug!("Return");
        let return_address = self.current_frame.return_address;
        debug!("From {:?}", self.current_frame);
        self.current_frame = self.stack.remove(self.stack.len() - 1);
        debug!("To {:?}", self.current_frame);
        match return_variable {
            Some(v) => self.set_variable(v, return_value, false)?,
            None => {}
        };

        Ok(return_address)
    }

    pub fn get_variable(&mut self, variable_number: u8, indirect: bool) -> Result<u16, InfocomError> {
        match variable_number {
            0 => {
                debug!("Read fron stack => ${:04x}", self.current_frame.peek()?);
                if indirect {
                    self.current_frame.peek()
                } else {
                    self.current_frame.pop()
                }
            },
            1..=15 => {
                debug!("Read local variable ${:02x} from {:?} => ${:04x}", variable_number - 1, self.current_frame.local_variables, self.current_frame.local_variables.get(variable_number as usize - 1).unwrap());
                match self.current_frame.local_variables.get(variable_number as usize - 1) {
                    Some(v) => Ok(*v),
                    None => Err(InfocomError::Memory(format!("Read of local variable ${:02x} that does not exist", variable_number - 1)))
                }
            }
            16..=255 => {
                let addr = self.global_variable_table_address + ((variable_number as usize - 16) * 2);
                debug!("Read global variable ${:02x} from ${:04x} => ${:04x}", variable_number - 16, addr, self.memory.get_word(addr)?);
                self.memory.get_word(addr)
            }
        }
    }

    pub fn set_variable(&mut self, variable_number: u8, value: u16, indirect: bool) -> Result<(), InfocomError> {
        match variable_number {
            0 => {
                debug!("Push ${:04x} to stack", value);
                // Replace the top of the stack when SP is and indirect variable reference
                if indirect {
                    self.current_frame.pop()?;
                }
                self.current_frame.push(value);
                debug!("{:?}", self.current_frame.stack);
                Ok(())
            },
            1..=15 => {
                debug!("Write ${:04x} to local variable ${:02x}", value, variable_number - 1);
                if self.current_frame.local_variables.len() >= variable_number as usize {
                    self.current_frame.local_variables[variable_number as usize - 1] = value;
                    Ok(())
                } else {
                    Err(InfocomError::Memory(format!("Write to variable ${:02x} that does not exist", variable_number - 1)))
                }
            },
            16..=255 => {
                let addr = self.global_variable_table_address + ((variable_number as usize - 16) * 2);
                debug!("Write ${:04x} to global variable ${:02x} at ${:04x}", value, variable_number - 16, addr);
                self.memory.set_word(addr, value)
            }
        }
    }
}