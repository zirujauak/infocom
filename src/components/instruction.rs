use super::memory::{ MemoryMap, Version };
use super::InfocomError;
use super::state::FrameStack;
use super::object_table::ObjectTable;
use super::text::{ Decoder, Encoder };
use super::interface::{ Interface, StatusLineFormat };
use super::dictionary::Dictionary;

use log::debug;
use serde::{ Serialize };
use std::collections::HashSet;
use std::iter::FromIterator;

#[derive(Debug, Serialize)]
enum OpcodeForm {
    Long,
    Short,
    Extended,
    Variable
}

impl From<u8> for OpcodeForm {
    fn from(opcode: u8) -> OpcodeForm {
        if opcode == 0xBE {
            OpcodeForm::Extended
        } else {
            match (opcode >> 6) & 0x3 {
                2 => OpcodeForm::Short,
                3 => OpcodeForm::Variable,
                _ => OpcodeForm::Long
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize)]
enum OperandType {
    LargeConstant,
    SmallConstant,
    Variable,
    Omitted
}

impl From<u8> for OperandType {
    fn from(v: u8) -> OperandType {
        match v & 0x3 {
            0 => OperandType::LargeConstant,
            1 => OperandType::SmallConstant,
            2 => OperandType::Variable,
            _ => OperandType::Omitted
        }
    }
}

#[derive(Serialize)]
pub struct Instruction {
    address: usize,
    form: OpcodeForm,
    opcode: u8,
    name: String,
    operand_types: Vec<OperandType>,
    operands: Vec<u16>,
    store_variable: Option<u8>,
    branch_offset: Option<BranchOffset>,
    next_pc: usize
}

use std::fmt;

fn format_variable(operand: u8) -> String {
    match operand {
        0 => String::from("(SP)"),
        1..=15 => format!("L{:02x}", operand - 1),
        _ => format!("G{:02x}", operand - 16)
    }
}

impl fmt::Debug for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let store = match self.store_variable {
            Some(v) => format!(" S:{}", format_variable(v)),
            _ => String::new()
        };

        let branch = match &self.branch_offset {
            Some(b) => match b.return_value {
                Some(v) => format!(" B: {} -> RET {}", b.condition, v),
                _ => format!(" B:[{}]->${:06x}", b.condition, b.address.unwrap())
            },
            _ => String::new()
        };

        let mut args = String::new();
        for i in 0..self.operands.len() {
            if i > 0 {
                args.push_str(",");
            }
            match self.operand_types[i] {
                OperandType::SmallConstant => args.push_str(&format!("#{:02x}", self.operands[i])),
                OperandType::LargeConstant => args.push_str(&format!("#{:04x}", self.operands[i])),
                OperandType::Variable => args.push_str(&format_variable(self.operands[i] as u8)),
                _ => {}
            }
        }

        f.write_fmt(format_args!("${:06x}: {} {} {}{}", self.address, self.name, args, store, branch))
    }
}

#[derive(Default, Serialize)]
pub struct InstructionResult {
    store_value: Option<u16>,
    branch_condition: Option<bool>,
    next_pc: Option<usize>
}

impl fmt::Debug for InstructionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let store_value = match self.store_value {
            Some(v) => format!(" S=>${:04x}", v),
            _ => String::new()
        };

        let branch_condition = match self.branch_condition {
            Some(b) => format!(" B=>{}", b),
            _ => String::new()
        };

        f.write_fmt(format_args!("{}{}", store_value, branch_condition))
    }
}

impl Instruction {
    fn get_argument(&self, state: &mut FrameStack, index: usize) -> Result<u16,InfocomError> {
        Ok(match self.operand_types[index] {
            OperandType::SmallConstant => self.operands[index] & 0xFF,
            OperandType::LargeConstant => self.operands[index],
            OperandType::Variable => {
                let var = (self.operands[index] & 0xFF) as u8;
                state.get_variable(var, false)?
            },
            OperandType::Omitted => return Err(InfocomError::Memory(format!("Operand with type 'Omitted'")))
        })
    }

    fn get_indirect_variable_reference(&self, state: &mut FrameStack, index: usize) -> Result<u8,InfocomError> {
        debug!("indirect reference: {:?} ${:02x}", self.operand_types[index], self.operands[index]);
        Ok(match self.operand_types[index] {
            OperandType::SmallConstant | OperandType::LargeConstant => (self.operands[index] & 0xFF) as u8,
            OperandType::Variable => state.get_variable((self.operands[index] & 0xFF) as u8, true)? as u8,
            OperandType::Omitted => return Err(InfocomError::Memory(format!("Operand with type 'Omitted'")))
        })
    }

    // 2OP
    fn je(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let a = self.get_argument(state, 0)?;
        for i in 1..self.operands.len() {
            let b = self.get_argument(state, i)?;
            debug!("JE: ${:04x} ${:04x}", a, b);
            if a == b {
                return Ok(InstructionResult { branch_condition: Some(true), ..Default::default() })
            }
        }

        return Ok(InstructionResult { branch_condition: Some(false), ..Default::default() })
    }

    fn jg(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let a = self.get_argument(state, 0)? as i16;
        for i in 1..self.operands.len() {
            let b = self.get_argument(state, i)? as i16;
            if a <= b {
                return Ok(InstructionResult { branch_condition: Some(false), ..Default::default() })
            }
        }

        return Ok(InstructionResult { branch_condition: Some(true), ..Default::default() })
    }

    fn jl(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let a = self.get_argument(state, 0)? as i16;
        for i in 1..self.operands.len() {
            let b = self.get_argument(state, i)? as i16;
            if a >= b {
                return Ok(InstructionResult { branch_condition: Some(false), ..Default::default() })
            }
        }

        return Ok(InstructionResult { branch_condition: Some(true), ..Default::default() })
    }

    fn dec_chk(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let var = self.get_indirect_variable_reference(state, 0)?;
        let var_value = state.get_variable(var, true)? as i16 - 1;
        state.set_variable(var, var_value as u16, true)?;
        let value = self.get_argument(state, 1)? as i16;
        Ok(InstructionResult { branch_condition: Some(var_value < value), ..Default::default() })   
    }

    fn inc_chk(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let var = self.get_indirect_variable_reference(state, 0)?;
        let var_value = state.get_variable(var, true)? as i16 + 1;
        state.set_variable(var, var_value as u16, true)?;
        let value = self.get_argument(state, 1)? as i16;
        Ok(InstructionResult { branch_condition: Some(var_value > value), ..Default::default() })   
    }

    fn jin(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let a = self.get_argument(state, 0)? as usize;
        let b = self.get_argument(state, 1)?;
        let ot = ObjectTable::new(&state.get_memory())?;
        let o = ot.get_object(&state.get_memory(), a)?;

        Ok(InstructionResult { branch_condition: Some(o.get_parent() == b), ..Default::default() })
    }

    fn test(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let bitmap = self.get_argument(state, 0)?;
        let flags = self.get_argument(state, 1)?;

        Ok(InstructionResult { branch_condition: Some(bitmap & flags == flags), ..Default::default() })
    }

    fn or(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut r:u16 = self.get_argument(state, 0)?;
        for i in 1..self.operands.len() {
            r |= self.get_argument(state, i)?;
        }

        Ok(InstructionResult { store_value: Some(r), ..Default::default() })
    }

    fn and(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut r:u16 = self.get_argument(state, 0)?;
        for i in 1..self.operands.len() {
            r &= self.get_argument(state, i)?;
        }

        Ok(InstructionResult { store_value: Some(r), ..Default::default() })
    }

    fn test_attr(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let attr = self.get_argument(state, 1)?;
        let ot = ObjectTable::new(&state.get_memory())?;

        Ok(InstructionResult { branch_condition: Some(ot.has_attribute(state.get_memory(), object as usize, attr as usize)?), ..Default::default() })
    }
    
    fn set_attr(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let attr = self.get_argument(state, 1)?;
        let mut ot = ObjectTable::new(&state.get_memory())?;
        ot.set_attribute(state, object as usize, attr as usize)?;

        Ok(InstructionResult::default())
    }
        
    fn clear_attr(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let attr = self.get_argument(state, 1)?;
        let mut ot = ObjectTable::new(&state.get_memory())?;
        ot.clear_attribute(state, object as usize, attr as usize)?;

        Ok(InstructionResult::default())
    }

    fn store(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let var = self.get_indirect_variable_reference(state, 0)?;
        let value = self.get_argument(state, 1)?;
        state.set_variable(var, value, false)?;
        Ok(InstructionResult::default())   
    }

    fn insert_obj(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let destination = self.get_argument(state, 1)?;
        let mut ot = ObjectTable::new(state.get_memory())?;
        ot.insert_object(state, object as usize, destination as usize)?;

        Ok(InstructionResult::default())
    }

    fn loadw(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let array = self.get_argument(state, 0)?;
        let index = self.get_argument(state, 1)?;
        let value = state.get_memory().get_word(array as usize + (index as usize * 2))?;

        Ok(InstructionResult { store_value: Some(value), ..Default::default() })
    }

    fn loadb(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let array = self.get_argument(state, 0)? as usize;
        let index = self.get_argument(state, 1)? as usize;
        let value = state.get_memory().get_byte(array + index)?;

        Ok(InstructionResult { store_value: Some(value as u16), ..Default::default() })
    }

    fn get_prop(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let property = self.get_argument(state, 1)? as usize;
        let ot = ObjectTable::new(state.get_memory())?;
        let value = ot.get_property_value(state.get_memory(), object, property)?;

        Ok(InstructionResult { store_value: Some(value), ..Default::default() })
    }

    fn get_prop_addr(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let property = self.get_argument(state, 1)? as usize;
        let ot = ObjectTable::new(state.get_memory())?;
        let value = ot.get_property_address(state.get_memory(), object, property)?;

        Ok(InstructionResult { store_value: Some(value as u16), ..Default::default() })
    }

    fn get_next_prop(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let property = self.get_argument(state, 1)? as usize;
        let ot = ObjectTable::new(state.get_memory())?;
        let value = ot.get_next_property(state.get_memory(), object, property)?;

        Ok(InstructionResult { store_value: Some(value as u16), ..Default::default() })
    }

    fn add(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut result:i16 = 0;
        for i in 0..self.operands.len() {
            let arg = self.get_argument(state, i)?;
            debug!("Add ${:04x} to ${:04x}", arg, result);
            result = result + arg as i16;
        }
        
        Ok(InstructionResult { store_value: Some(result as u16), ..Default::default() })
    }

    fn sub(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut result:i16 = self.get_argument(state, 0)? as i16;
        for i in 1..self.operands.len() {
            let arg = self.get_argument(state, i)?;
            debug!("Sub ${:04x} from ${:04x}", arg, result);
            result = result - arg as i16;
        }
        
        Ok(InstructionResult { store_value: Some(result as u16), ..Default::default() })
    }
    
    fn mul(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut result:i16 = self.get_argument(state, 0)? as i16;
        for i in 1..self.operands.len() {
            let arg = self.get_argument(state, i)?;
            debug!("Mul ${:04x} by ${:04x}", arg, result);
            result = result * arg as i16;
        }
        
        Ok(InstructionResult { store_value: Some(result as u16), ..Default::default() })
    }

    fn div(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut result:i16 = self.get_argument(state, 0)? as i16;
        for i in 1..self.operands.len() {
            let arg = self.get_argument(state, i)?;
            if arg == 0 {
                return Err(InfocomError::Memory(format!("Division by zero")));
            }
            debug!("Div ${:04x} by ${:04x}", arg, result);
            result = result / arg as i16;
        }
        
        Ok(InstructionResult { store_value: Some(result as u16), ..Default::default() })
    }

    fn modulo(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let mut result:i16 = self.get_argument(state, 0)? as i16;
        for i in 1..self.operands.len() {
            let arg = self.get_argument(state, i)?;
            if arg == 0 {
                return Err(InfocomError::Memory(format!("Modulo by zero")));
            }
            debug!("Mod ${:04x} by ${:04x}", arg, result);
            result = result.rem_euclid(arg as i16);
        }
        
        Ok(InstructionResult { store_value: Some(result as u16), ..Default::default() })
    }

    fn call_2s(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let routine = self.get_argument(state, 0)?;
        let arg = self.get_argument(state, 1)?;

        let next_pc = state.call(routine, vec![arg], self.store_variable, self.next_pc)?;

        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn call_2n(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let routine = self.get_argument(state, 0)?;
        let arg = self.get_argument(state, 1)?;

        let next_pc = state.call(routine, vec![arg], None, self.next_pc)?;

        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn set_colour(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("set_colour not implemented yet")))
    }

    fn throw(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("throw not implemented yet")))
    }

    // 1OP
    fn jz(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let a = self.get_argument(state, 0)?;

        Ok(InstructionResult { branch_condition: Some(a == 0), ..Default::default() })
    }

    fn get_sibling(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let ot = ObjectTable::new(state.get_memory())?;
        let o = ot.get_object(state.get_memory(), object as usize)?;
        Ok(InstructionResult { store_value: Some(o.get_sibling()), branch_condition: Some(o.get_sibling() != 0), ..Default::default() })
    }

    fn get_child(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let ot = ObjectTable::new(state.get_memory())?;
        let o = ot.get_object(state.get_memory(), object as usize)?;
        Ok(InstructionResult { store_value: Some(o.get_child()), branch_condition: Some(o.get_child() != 0), ..Default::default() })
    }

    fn get_parent(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)?;
        let ot = ObjectTable::new(state.get_memory())?;
        let o = ot.get_object(state.get_memory(), object as usize)?;
        Ok(InstructionResult { store_value: Some(o.get_parent()), ..Default::default() })
    }

    fn get_prop_len(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let ot = ObjectTable::new(state.get_memory())?;

        Ok(InstructionResult { store_value: Some(ot.get_property_len(state.get_memory(), object)? as u16), ..Default::default() })
    }

    fn inc(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let var = self.get_indirect_variable_reference(state, 0)?;
        let var_value = state.get_variable(var, true)? as i16 + 1;
        state.set_variable(var, var_value as u16, true)?;
        Ok(InstructionResult::default())
    }

    fn dec(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let var = self.get_indirect_variable_reference(state, 0)?;
        let var_value = state.get_variable(var, true)? as i16 - 1;
        state.set_variable(var, var_value as u16, true)?;
        Ok(InstructionResult::default())
    }

    fn print_addr(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let addr = self.get_argument(state, 0)? as usize;
        let decoder = Decoder::new(state.get_memory())?;
        let string = decoder.decode(addr)?;
        print!("{}", string);

        Ok(InstructionResult::default())
    }

    fn call_1s(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let routine = self.get_argument(state, 0)?;

        let next_pc = state.call(routine, vec![], self.store_variable, self.next_pc)?;

        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn remove_obj(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let mut ot = ObjectTable::new(state.get_memory())?;
        ot.remove_object(state, object)?;

        Ok(InstructionResult::default())
    }

    fn print_obj(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let ot = ObjectTable::new(state.get_memory())?;
        let o = ot.get_object(&mut state.get_memory(), object)?;
        interface.print(&o.get_short_name());

        Ok(InstructionResult::default())
    }

    fn ret(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let value = self.get_argument(state, 0)?;
        let next_pc = state.return_from(value)?;

        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn jump(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let label = self.get_argument(state, 0)? as i16;
        let address = self.next_pc as isize + label as isize - 2;
        debug!("JUMP: {} -> {}", label, self.next_pc);

        Ok(InstructionResult { next_pc: Some(address as usize), ..Default::default() })
    }

    fn print_paddr(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        let packed_address = self.get_argument(state, 0)?;
        let address = state.unpack_address(packed_address)?;
        let decoder = Decoder::new(state.get_memory())?;
        let string = decoder.decode(address)?;
        interface.print(&string);

        Ok(InstructionResult::default())
    }

    fn load(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let var = self.get_indirect_variable_reference(state, 0)?;
        let var_value = state.get_variable(var, true)?;

        Ok(InstructionResult { store_value: Some(var_value), ..Default::default() })
    }

    // Also VAR:18 for version 5+
    fn not(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let value = self.get_argument(state, 0)?;
        let result = !value;

        Ok(InstructionResult { store_value: Some(result), ..Default::default() })
    }

    fn call_1n(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let routine = self.get_argument(state, 0)?;

        let next_pc = state.call(routine, vec![], None, self.next_pc)?;

        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    // 0OP
    fn rtrue(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let next_pc = state.return_from(1)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn rfalse(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let next_pc = state.return_from(0)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn print(&self, state: &FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        let address = self.address + 1;
        let decoder = Decoder::new(state.get_memory())?;
        let string = decoder.decode(address)?;
        interface.print(&string);

        Ok(InstructionResult::default())
    }

    fn print_ret(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        let address = self.address + 1;
        let decoder = Decoder::new(state.get_memory())?;
        let string = decoder.decode(address)?;
        interface.print(&string);
        interface.new_line();

        let next_pc = state.return_from(1)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn nop(&self, state: &FrameStack) -> Result<InstructionResult,InfocomError> {
        debug!("NOP");
        Ok(InstructionResult::default())
    }

    fn save_v1(&self, state: &FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("save_v1 not implemented yet")))
    }

    fn save_v4(&self, state: &FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("save_v4 not implemented yet")))
    }

    fn restore_v1(&self, state: &FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("restore_v1 not implemented yet")))
    }

    fn restore_v4(&self, state: &FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("restore_v4 not implemented yet")))
    }

    fn restart(&self, state: &FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("restart not implemented yet")))
    }

    fn ret_popped(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let value = state.get_variable(0, false)?;
        let next_pc = state.return_from(value)?;

        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }
    
    fn pop(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        state.get_variable(0, false)?;
        
        Ok(InstructionResult::default())
    }

    fn catch(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("catch not implemented yet")))
    }

    fn quit(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("quit not implemented yet")))
    }

    fn new_line(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        interface.new_line();

        Ok(InstructionResult::default())
    }

    fn show_status(&self, state: &mut FrameStack, interface: &mut Interface) -> Result<InstructionResult,InfocomError> {
        let v1 = state.get_variable(17, false)? as i16;
        let v2 = state.get_variable(18, false)?;
        let name_obj = state.get_variable(16, false)? as usize;
        let o = ObjectTable::new(state.get_memory())?.get_object(state.get_memory(), name_obj)?;
        let status_type = match state.get_memory().version {
            Version::V(3) => {
                let flags1 = state.get_memory().get_byte(0x01)?;
                if flags1 & 0x02 == 0x02 {
                    StatusLineFormat::TIMED
                } else {
                    StatusLineFormat::SCORED
                }
            },
            _ => StatusLineFormat::SCORED
        };

        interface.status_line(&o.get_short_name(), status_type, v1, v2);
        Ok(InstructionResult::default())    
    }

    fn verify(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("verify not implemented yet")))
    }

    fn piracy(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        debug!("PIRACY: {:?}", self.branch_offset.as_ref().unwrap());

        Ok(InstructionResult { branch_condition: Some(self.branch_offset.as_ref().unwrap().condition), ..Default::default() })
    }

    // VAR
    fn call(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let packed_address = self.get_argument(state, 0)?;
        let mut args:Vec<u16> = Vec::new();
        for i in 1..self.operands.len() {
            args.push(self.get_argument(state, i)?);
        }

        let next_pc = state.call(packed_address, args, self.store_variable, self.next_pc)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn storew(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let array = self.get_argument(state, 0)? as usize;
        let word_index = self.get_argument(state, 1)? as usize;
        let value = self.get_argument(state, 2)?;

        state.set_word(array + (2 * word_index), value)?;

        Ok(InstructionResult::default())
    }

    fn storeb(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let array = self.get_argument(state, 0)? as usize;
        let byte_index = self.get_argument(state, 1)? as usize;
        let value = (self.get_argument(state, 2)? as u8) & 0xFF;

        state.set_byte(array + byte_index, value)?;

        Ok(InstructionResult::default())
    }

    fn put_prop(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let object = self.get_argument(state, 0)? as usize;
        let property = self.get_argument(state, 1)? as usize;
        let value = self.get_argument(state, 2)?;
        let mut ot = ObjectTable::new(state.get_memory())?;

        ot.put_property_data(state, object, property, value)?;
        Ok(InstructionResult::default())
    }

    fn sread_v1(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        self.show_status(state, interface)?;
        // let v2 = state.get_variable(18, false)?;
        // let name_obj = state.get_variable(16, false)? as usize;
        // let o = ObjectTable::new(state.get_memory())?.get_object(state.get_memory(), name_obj)?;
        // let status_type = match state.get_memory().version {
        //     Version::V(3) => {
        //         let flags1 = state.get_memory().get_byte(0x01)?;
        //         if flags1 & 0x02 == 0x02 {
        //             StatusLineFormat::TIMED
        //         } else {
        //             StatusLineFormat::SCORED
        //         }
        //     },
        //     _ => StatusLineFormat::SCORED
        // };

        // interface.status_line(&o.get_short_name(), status_type, v1, v2);

        let text_buffer = self.get_argument(state, 0)? as usize;
        let parse_buffer = self.get_argument(state, 1)? as usize;
        let max_chars = state.get_memory().get_byte(text_buffer)? as usize - 1;

        debug!("Text buffer: ${:04x} for ${:02x} bytes", text_buffer, max_chars);

        let mut input = interface.read(HashSet::from_iter(vec!['\n', '\r']), max_chars);
        // Remove the terminating character from the buffer...
        let terminator = input.pop();
        debug!("Input: {}", input);

        let encoder = Encoder::new(state.get_memory())?;
        let mut input_bytes = encoder.to_bytes(&input);
        // ...and replace it with a 0 byte
        input_bytes.push(0);

        // Byte 1 of the buffer is the number of characters read
        state.set_byte(text_buffer + 1, input.len() as u8)?;

        // Byte 2 onward is the text with a '\0' terminator.
        for (i, c) in input_bytes.iter().enumerate() {
            state.set_byte(text_buffer + i + 2, *c)?;
        }

        let max_words = state.get_memory().get_byte(parse_buffer)?;
        debug!("Parse buffer: ${:04x} for ${:02x} words", parse_buffer, max_words);

        let dic = Dictionary::new(state.get_memory())?;
        dic.analyze_text(state, &input, parse_buffer)?;
        // state.set_byte(parse_buffer + 1, 1)?;
        // state.set_word(parse_buffer + 2, 0)?;
        // state.set_byte(parse_buffer + 4, input.len() as u8)?;
        // state.set_byte(parse_buffer + 5, 1)?;

        // // TODO: Parse words
        Ok(InstructionResult::default())
    }

    fn sread_v4(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("sread not implemented yet")))
    }

    fn aread(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("aread not implemented yet")))
    }

    fn print_char(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        let z = self.get_argument(state, 0)?;
        let d = Decoder::new(state.get_memory())?;
        interface.print(&format!("{}", d.zscii_to_char(z)?));

        Ok(InstructionResult::default())
    }

    fn print_num(&self, state: &mut FrameStack, interface: &mut dyn Interface) -> Result<InstructionResult,InfocomError> {
        let value = self.get_argument(state, 0)? as i16;
        interface.print(&format!("{}", value));

        Ok(InstructionResult::default())
    }

    fn random(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let range = self.get_argument(state, 0)?;
        let value = state.random(range)?;
        Ok(InstructionResult { store_value: Some(value), ..Default::default() })
    }

    fn push(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let value = self.get_argument(state, 0)?;
        state.current_frame.push(value);

        Ok(InstructionResult::default())
    }

    fn pull(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let variable = self.get_indirect_variable_reference(state, 0)?;
        let value = state.current_frame.pop()?;
        state.set_variable(variable, value, false)?;

        Ok(InstructionResult::default())
    }

    fn split_window(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("split_window not implemented yet")))
    }

    fn set_window(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("set_window not implemented yet")))
    }

    fn call_vs2(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let packed_address = self.get_argument(state, 0)?;
        let mut args:Vec<u16> = Vec::new();
        for i in 1..self.operands.len() {
            args.push(self.get_argument(state, i)?);
        }

        let next_pc = state.call(packed_address, args, self.store_variable, self.next_pc)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn erase_window(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("erase_window not implemented yet")))
    }

    fn erase_line(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("erase_line not implemented yet")))
    }

    fn set_cursor(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("set_cursor not implemented yet")))
    }

    fn get_cursor(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("get_cursor not implemented yet")))
    }

    fn set_text_style(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("set_text_style not implemented yet")))
    }

    fn buffer_mode(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("buffer_mode not implemented yet")))
    }

    fn output_stream(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("output_stream not implemented yet")))
    }

    fn input_stream(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("input_stream not implemented yet")))
    }

    fn sound_effect(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("sound_effect not implemented yet")))
    }

    fn read_char(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("read_char not implemented yet")))
    }

    fn scan_table(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("scan_table not implemented yet")))
    }

    fn call_vn(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let packed_address = self.get_argument(state, 0)?;
        let mut args:Vec<u16> = Vec::new();
        for i in 1..self.operands.len() {
            args.push(self.get_argument(state, i)?);
        }

        let next_pc = state.call(packed_address, args, None, self.next_pc)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn call_vn2(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        let packed_address = self.get_argument(state, 0)?;
        let mut args:Vec<u16> = Vec::new();
        for i in 1..self.operands.len() {
            args.push(self.get_argument(state, i)?);
        }

        let next_pc = state.call(packed_address, args, None, self.next_pc)?;
        Ok(InstructionResult { next_pc: Some(next_pc), ..Default::default() })
    }

    fn tokenise(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("tokenise not implemented yet")))
    }

    fn encode_text(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("encode_text not implemented yet")))
    }

    fn copy_table(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("copy_table not implemented yet")))
    }

    fn print_table(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("print_table not implemented yet")))
    }

    fn check_arg_count(&self, state: &mut FrameStack) -> Result<InstructionResult,InfocomError> {
        Err(InfocomError::Memory(format!("check_arg_count not implemented yet")))
    }

    pub fn execute<T>(&mut self, state: &mut FrameStack, interface: &mut T) -> Result<usize,InfocomError> 
    where
        T: Interface
    {
        debug!("{:?}", self);
        let result = match state.get_memory().version {
            Version::V(3) => {
                if self.opcode < 0x80 || (self.opcode > 0xBf && self.opcode < 0xE0) {
                    match self.opcode & 0x1F {
                        0x01 => self.je(state),
                        0x02 => self.jl(state),
                        0x03 => self.jg(state),
                        0x04 => self.dec_chk(state),
                        0x05 => self.inc_chk(state),
                        0x06 => self.jin(state),
                        0x07 => self.test(state),
                        0x08 => self.or(state),
                        0x09 => self.and(state),
                        0x0A => self.test_attr(state),
                        0x0B => self.set_attr(state),
                        0x0C => self.clear_attr(state),
                        0x0D => self.store(state),
                        0x0E => self.insert_obj(state),
                        0x0F => self.loadw(state),
                        0x10 => self.loadb(state),
                        0x11 => self.get_prop(state),
                        0x12 => self.get_prop_addr(state),
                        0x13 => self.get_next_prop(state),
                        0x14 => self.add(state),
                        0x15 => self.sub(state),
                        0x16 => self.mul(state),
                        0x17 => self.div(state),
                        0x18 => self.modulo(state),
                        _ => Err(InfocomError::Memory(format!("Unimplemented opcode ${:02x}", self.opcode)))
                    }
                } else if self.opcode > 0x7F && self.opcode < 0xB0 {
                    match self.opcode & 0xF {
                        0x00 => self.jz(state),
                        0x01 => self.get_sibling(state),
                        0x02 => self.get_child(state),
                        0x03 => self.get_parent(state),
                        0x04 => self.get_prop_len(state),
                        0x05 => self.inc(state),
                        0x06 => self.dec(state),
                        0x07 => self.print_addr(state),
                        0x09 => self.remove_obj(state),
                        0x0A => self.print_obj(state, interface),
                        0x0B => self.ret(state),
                        0x0C => self.jump(state),
                        0x0D => self.print_paddr(state, interface),
                        0x0E => self.load(state),
                        0x0F => self.not(state),
                        _ => Err(InfocomError::Memory(format!("Unimplemented opcode ${:02x}", self.opcode)))
                    }
                } else if self.opcode > 0xAF && self.opcode < 0xC0 {
                    match self.opcode & 0xF {
                        0x00 => self.rtrue(state),
                        0x01 => self.rfalse(state),
                        0x02 => self.print(state, interface),
                        0x03 => self.print_ret(state, interface),
                        0x04 => self.nop(state),
                        0x05 => self.save_v1(state),
                        0x06 => self.restore_v1(state),
                        0x07 => self.restart(state),
                        0x08 => self.ret_popped(state),
                        0x09 => self.pop(state),
                        0x0A => self.quit(state),
                        0x0B => self.new_line(state, interface),
                        0x0C => self.show_status(state, interface),
                        0x0D => self.verify(state),
                        _ => Err(InfocomError::Memory(format!("Unimplemented opcode ${:02x}", self.opcode)))
                    }
                } else {
                    match self.opcode & 0x1F {
                        0x00 => self.call(state),
                        0x01 => self.storew(state),
                        0x02 => self.storeb(state),
                        0x03 => self.put_prop(state),
                        0x04 => self.sread_v1(state, interface),
                        0x05 => self.print_char(state, interface),
                        0x06 => self.print_num(state, interface),
                        0x07 => self.random(state),
                        0x08 => self.push(state),
                        0x09 => self.pull(state),
                        0x0A => self.split_window(state),
                        0x0B => self.set_window(state),
                        0x13 => self.output_stream(state),
                        0x14 => self.input_stream(state),
                        0x15 => self.sound_effect(state),
                        _ => Err(InfocomError::Memory(format!("Unimplemented opcode ${:02x}", self.opcode)))

                    }
                } 
            },
            _ => Err(InfocomError::Memory(format!("Unimplemented verison {:?}", state.get_memory().version)))
        }?;

        match result.store_value {
            Some(_) => debug!("{:?}", result),
            _ => match result.branch_condition {
                Some(_) => debug!("{:?}", result),
                _ => {}
            }
        }

        // CALL instructions have a store_variable, but do not store a result
        if let Some(var) = self.store_variable {
            if let Some(store_value) = result.store_value {
                state.set_variable(var, store_value, false)?;
            }
        }

        if let Some(offset) = &self.branch_offset {
            if result.branch_condition.unwrap() == offset.condition {
                if let Some(ret) = offset.return_value {
                    return state.return_from(ret as u16)
                }
                return Ok(offset.address.unwrap())
            }
        }

        if let Some(next_pc) = result.next_pc {
            Ok(next_pc)
        } else {
            Ok(self.next_pc)
        }
    }
}

fn read_byte(mem: &Vec<u8>, address: usize) -> u8 {
    mem[address]
}

fn read_word(mem: &Vec<u8>, address: usize) -> u16 {
    let high = mem[address];
    let low = mem[address + 1];

    (((high as u16) << 8) & 0xFF00) | (low as u16 & 0xFF)
}

fn get_store_variable(mem: &Vec<u8>, address: usize, opcode: u8, form: &OpcodeForm) -> Option<u8> {
    match form {
        OpcodeForm::Extended => {
            match opcode {
              1 | 2 | 3 | 4 | 9 | 10 | 19 | 29 => { Some(read_byte(mem, address)) },
              _ => None
            }
        },
        _ => match opcode {
            // Long 2OP, Variable 2OP
            0x00..=0x7F | 0xC0..=0xDF => {
                match opcode & 0x1F {
                    8 | 9 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 => { Some(read_byte(mem, address)) }
                    _ => None
                }
            },
            // Short 1OP
            0x80..=0xAF => {
                match opcode & 0xF {
                    1 | 2 | 3 | 4| 8 | 14 => { Some(read_byte(mem, address)) },
                    15 => if read_byte(mem, 0) < 5 {
                        Some(read_byte(mem, address))
                    } else {
                        None
                    }
                    _ => None,
                }
            },
            // Short 0OP
            0xB0..=0xBF => {
                match opcode & 0xF {
                    5 | 6 => if read_byte(mem, 0) == 4 {
                        Some(read_byte(mem, address))
                    } else {
                        None
                    },
                    9 => if read_byte(mem, 0) > 4 { 
                        Some(read_byte(mem, address))
                    } else {
                        None 
                    },
                    _ => None,
                }
            },
            // Variable VAR
            0xE0..=0xFF => {
                match opcode & 0x1F {
                    0 | 7 | 12 | 22 | 23 | 24 => { Some(read_byte(mem, address)) },
                    4 => if read_byte(mem, 0) > 4 {
                        Some(read_byte(mem, address))
                    } else {
                        None
                    },
                    9 => if read_byte(mem, 0) == 6 {
                        Some(read_byte(mem, address)) 
                    } else {
                        None
                    },
                    _ => None
                }
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct BranchOffset {
    size: usize,
    condition: bool,
    return_value: Option<u8>,
    address: Option<usize>,
}

fn decode_branch_offset(mem: &Vec<u8>, address: usize) -> BranchOffset {
    let b1 = read_byte(mem, address);
    let condition = b1 & 0x80 == 0x80;
    if b1 & 0x40 == 0x40 {
        let offset = b1 & 0x3F;
        match offset {
            0 => BranchOffset { size: 1, condition, return_value: Some(0), address: None },
            1 => BranchOffset { size: 1, condition, return_value: Some(1), address: None },
            _ => BranchOffset { size: 1, condition, return_value: None, address: Some((address as isize + offset as isize - 1) as usize) }
        }
    } else {
        let mut high = b1 & 0x3F;
        if high & 0x20 == 0x20 {
            high |= 0xC0;
        }
        let low = read_byte(mem, address + 1);
        let offset:i16 = ((((high as u16) << 8) & 0xFF00) | (low as u16 & 0xFF)) as i16;
        match offset {
            0 => BranchOffset { size: 2, condition, return_value: Some(0), address: None },
            1 => BranchOffset { size: 2, condition, return_value: Some(1), address: None },
            _ => BranchOffset { size: 2, condition, return_value: None, address: Some((address as isize + offset as isize) as usize) }
        }
    }
}

fn get_branch_offset(mem: &Vec<u8>, address: usize, opcode: u8, form: &OpcodeForm) -> Option<BranchOffset> {
    match form {
        OpcodeForm::Extended => {
            match opcode {
                6 | 24 | 27 => { Some(decode_branch_offset(mem, address)) },
                _ => None
            }
        }, 
        _ => match opcode {
            // Long 2OP, Variable 2OP
            0x00..=0x7F | 0xC0..=0xDF => {
                match opcode & 0x1F {
                    1 | 2 | 3 | 4 | 5 | 6 | 7 | 10 => { Some(decode_branch_offset(mem, address)) },
                    _ => None
                }
            },
            // Short 1OP
            0x80..=0xAF => {
                match opcode & 0xF {
                    0 | 1 | 2 => { Some(decode_branch_offset(mem, address)) },
                    _ => None,
                }
            },
            // Short 0OP
            0xB0..=0xBF => {
                match opcode & 0xF {
                    13 | 15 => { Some(decode_branch_offset(mem, address)) },
                    5 | 6 => if read_byte(mem, 0) < 4 {
                        { Some(decode_branch_offset(mem, address)) }
                    } else {
                        None
                    },
                    _ => None,
                }
            },
            // Variable VAR
            0xE0..=0xFF => {
                match opcode & 0x1F {
                    17 | 31 => { Some(decode_branch_offset(mem, address)) },
                    _ => None
                }
            }
        }
    }
}

fn get_literal_string(mem: &Vec<u8>, address: usize, opcode: u8, form: &OpcodeForm) -> Option<usize> {
    match form {
        OpcodeForm::Extended => None,
        _ => match opcode {
            0xB2 | 0xB3 => {
                let mut size = 0;
                loop {
                    let v = read_word(mem, address + size);
                    size += 2;
                    if v & 0x8000 == 0x8000 {
                        break;
                    }
                }
                Some(size)
            },
            _ => None
        }
    }
}

pub fn decode_instruction(state: &FrameStack, address: usize) -> Result<Instruction, InfocomError> {
    let mem = state.get_memory().get_memory();
    let mut opcode_byte = read_byte(&mem, address);
    let mut ext_opcode:Option<u8> = None;
    let form = OpcodeForm::from(opcode_byte);
    let mut operand_types:Vec<OperandType> = Vec::new();
    let mut operands:Vec<u16> = Vec::new();

    let mut skip = 1;
    match form {
        OpcodeForm::Long => {
            if opcode_byte & 0x40 == 0x40 {
                operand_types.push(OperandType::Variable);
            } else {
                operand_types.push(OperandType::SmallConstant);
            }
            if opcode_byte & 0x20 == 0x20 {
                operand_types.push(OperandType::Variable);
            } else {
                operand_types.push(OperandType::SmallConstant);
            }
        },
        OpcodeForm::Short => {
            let ot = OperandType::from(opcode_byte >> 4);
            match ot {
                OperandType::Omitted => {},
                _ => operand_types.push(ot)
            }
        },
        OpcodeForm::Variable => {
            let types_1 = read_byte(&mem, address + 1);
            let oc = opcode_byte & 0x1F;

            // First operand type byte
            for i in 0..4 {
                let t = types_1 >> (6 - (i * 2));
                let ot = OperandType::from(t);
                match ot {
                    OperandType::Omitted => break,
                    _ => operand_types.push(ot)
                }
            }

            skip += 1;

            // Optional second operand type byte
            if oc == 12 || oc == 26 {
                let types_2 = read_byte(&mem, address + 2);
                for i in 0..4 {
                    let t = types_2 >> (6 - (i * 2));
                    let ot = OperandType::from(t);
                    match ot {
                        OperandType::Omitted => break,
                        _ => operand_types.push(ot)
                    }
                }
                skip += 1;
            }
        },
        OpcodeForm::Extended => {
            ext_opcode = Some(read_byte(&mem, address + 1));

            let types_1 = read_byte(&mem, address + 2);
            for i in 0..4 {
                let t = types_1 >> (6 - (i * 2));
                let ot = OperandType::from(t);
                match ot {
                    OperandType::Omitted => break,
                    _ => operand_types.push(ot)
                }
            }

            skip += 2;
        }
    };

    for operand_type in &operand_types {
        match operand_type {
            OperandType::SmallConstant | OperandType::Variable => {
                let v = read_byte(&mem, address + skip);
                operands.push(v as u16);
                skip += 1
            },
            OperandType::LargeConstant => {
                let v = read_word(&mem, address + skip);
                operands.push(v);
                skip += 2
            },
            OperandType::Omitted => {
                break
            }
        }
    }

    let store_variable = get_store_variable(&mem, address + skip, opcode_byte, &form);
    if let Some(_) = store_variable {
        skip = skip + 1;
    }

    let branch_offset = get_branch_offset(&mem, address + skip, opcode_byte, &form);
    if let Some(b) = &branch_offset {
        skip += b.size;
    }

    if let Some(l) = get_literal_string(&mem, address + skip, opcode_byte, &form) {
        skip += l;
    }
    
    let name = match opcode_byte {
        0x01 | 0x21 | 0x41 | 0x61 | 0xC1 => String::from("je"),
        0x02 | 0x22 | 0x42 | 0x62 | 0xC2 => String::from("jl"),
        0x03 | 0x23 | 0x43 | 0x63 | 0xC3 => String::from("jg"),
        0x04 | 0x24 | 0x44 | 0x64 | 0xC4 => String::from("dec_chk"),
        0x05 | 0x25 | 0x45 | 0x65 | 0xC5 => String::from("inc_chk"),
        0x06 | 0x26 | 0x46 | 0x66 | 0xC6 => String::from("jin"),
        0x07 | 0x27 | 0x47 | 0x67 | 0xC7 => String::from("test"),
        0x08 | 0x28 | 0x48 | 0x68 | 0xC8 => String::from("or"),
        0x09 | 0x29 | 0x49 | 0x69 | 0xC9 => String::from("and"),
        0x0A | 0x2A | 0x4A | 0x6A | 0xCA => String::from("test_attr"),
        0x0B | 0x2B | 0x4B | 0x6B | 0xCB => String::from("set_attr"),
        0x0C | 0x2C | 0x4C | 0x6C | 0xCC => String::from("clear_attr"),
        0x0D | 0x2D | 0x4D | 0x6D | 0xCD => String::from("store"),
        0x0E | 0x2E | 0x4E | 0x6E | 0xCE => String::from("insert_obj"),
        0x0F | 0x2F | 0x4F | 0x6F | 0xCF => String::from("loadw"),
        0x10 | 0x30 | 0x50 | 0x70 | 0xD0 => String::from("loadb"),
        0x11 | 0x31 | 0x51 | 0x71 | 0xD1 => String::from("get_prop"),
        0x12 | 0x32 | 0x52 | 0x72 | 0xD2 => String::from("get_prop_addr"),
        0x13 | 0x33 | 0x53 | 0x73 | 0xD3 => String::from("get_next_prop"),
        0x14 | 0x34 | 0x54 | 0x74 | 0xD4 => String::from("add"),
        0x15 | 0x35 | 0x55 | 0x75 | 0xD5 => String::from("sub"),
        0x16 | 0x36 | 0x56 | 0x76 | 0xD6 => String::from("mul"),
        0x17 | 0x37 | 0x57 | 0x77 | 0xD7 => String::from("div"),
        0x18 | 0x38 | 0x58 | 0x78 | 0xD8 => String::from("mod"),
        0x19 | 0x39 | 0x59 | 0x79 | 0xD9 => String::from("call_2s"),
        0x1A | 0x3A | 0x5A | 0x7A | 0xDA => String::from("call_2n"),
        0x1B | 0x3B | 0x5B | 0x7B | 0xDB => String::from("set_colour"),
        0x1C | 0x3C | 0x5C | 0x7C | 0xDC => String::from("throw"),
        0x80 | 0x90 | 0xA0 => String::from("jz"),
        0x81 | 0x91 | 0xA1 => String::from("get_sibling"),
        0x82 | 0x92 | 0xA2 => String::from("get_child"),
        0x83 | 0x93 | 0xA3 => String::from("get_parent"),
        0x84 | 0x94 | 0xA4 => String::from("get_prop_len"),
        0x85 | 0x95 | 0xA5 => String::from("inc"),
        0x86 | 0x96 | 0xA6 => String::from("dec"),
        0x87 | 0x97 | 0xA7 => String::from("print_addr"),
        0x88 | 0x98 | 0xA8 => String::from("call_1s"),
        0x89 | 0x99 | 0xA9 => String::from("remove_obj"),
        0x8A | 0x9A | 0xAA => String::from("print_obj"),
        0x8B | 0x9B | 0xAB => String::from("ret"),
        0x8C | 0x9C | 0xAC => String::from("jump"),
        0x8D | 0x9D | 0xAD => String::from("print_paddr"),
        0x8E | 0x9E | 0xAE => String::from("load"),
        0x8F | 0x9F | 0xAF => match state.get_memory().version {
            Version::V(1) | Version::V(2) | Version::V(3) | Version::V(4) => String::from("not"),
            _ => String::from("call_1n")
        },
        0xB0 => String::from("rtrue"),
        0xB1 => String::from("rfalse"),
        0xB2 => String::from("print"),
        0xB3 => String::from("print_ret"),
        0xB4 => String::from("nop"),
        0xB5 => String::from("save"),
        0xB6 => String::from("restore"),
        0xB7 => String::from("restart"),
        0xB8 => String::from("ret_popped"),
        0xB9 => String::from("rtrue"),
        0xBA => String::from("quit"),
        0xBB => String::from("new_line"),
        0xBC => String::from("show_status"),
        0xBD => String::from("verify"),
        0xBE => {
            let mut s = String::from("EXT ");
            let default = format!("${:02x}", ext_opcode.unwrap());
            s.push_str(match ext_opcode.unwrap() {
                0x00 => "save",
                0x01 => "restore",
                0x02 => "log_shift",
                0x03 => "art_shift",
                0x04 => "set_font",
                0x05 => "draw_picture",
                0x06 => "picture_data",
                0x07 => "erase_picture",
                0x08 => "set_margins",
                0x09 => "save_undo",
                0x0A => "restore_undo",
                0x0B => "print_unicode",
                0x0C => "check_unicode",
                0x0D => "set_true_colour",
                0x10 => "move_window",
                0x11 => "window_size",
                0x12 => "window_style",
                0x13 => "get_wind_prop",
                0x14 => "scroll_window",
                0x15 => "pop_stack",
                0x16 => "read_mouse",
                0x17 => "mouse_window",
                0x18 => "push_stack",
                0x19 => "put_wind_prop",
                0x1A => "print_form",
                0x1B => "make_menu",
                0x1C => "picture_table",
                0x1D => "buffer_screen",
                _ => &default
            });
            s
        },
        0xBF => String::from("piracy"),
        0xE0 => match state.get_memory().version {
            Version::V(1) | Version::V(2) | Version::V(3) => String::from("call"),
            _ => String::from("call_vs")
        },
        0xE1 => String::from("storew"),
        0xE2 => String::from("storeb"),
        0xE3 => String::from("put"),
        0xE4 => match state.get_memory().version {
            Version::V(v) => {
                match v {
                    1..=4 => String::from("sread"),
                    _ => String::from("aread")
                }
            }
        },
        0xE5 => String::from("print_char"),
        0xE6 => String::from("print_num"),
        0xE7 => String::from("random"),
        0xE8 => String::from("push"),
        0xE9 => String::from("pull"),
        0xEA => String::from("split_window"),
        0xEB => String::from("set_window"),
        0xEC => String::from("call_vs2"),
        0xED => String::from("erase_window"),
        0xEE => String::from("erase_line"),
        0xEF => String::from("set_cursor"),
        0xF0 => String::from("get_cursor"),
        0xF1 => String::from("set_text_style"),
        0xF2 => String::from("buffer_mode"),
        0xF3 => String::from("output_stream"),
        0xF4 => String::from("input_stream"),
        0xF5 => String::from("sound_effect"),
        0xF6 => String::from("read_char"),
        0xF7 => String::from("scan_table"),
        0xF8 => String::from("not"),
        0xF9 => String::from("call_vn"),
        0xFA => String::from("call_vn2"),
        0xFB => String::from("tokenize"),
        0xFC => String::from("encode_text"),
        0xFD => String::from("copy_table"),
        0xFE => String::from("print_table"),
        0xFF => String::from("check_arg_count"),
        _ => format!("${:02x}", opcode_byte)
    };

    if let Some(o) = ext_opcode {
        opcode_byte = o;
    }

    Ok(Instruction { address, name, form, opcode: opcode_byte, operand_types, operands, store_variable, branch_offset, next_pc: address + skip })
}
