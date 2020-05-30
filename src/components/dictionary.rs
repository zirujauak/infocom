use std::collections::HashSet;
use log::debug;

use super::InfocomError;
use super::memory::MemoryMap;
use super::text::{ Decoder, Encoder };
use super::state::FrameStack;

pub struct Dictionary {
    address: usize,
    separators: HashSet<char>,
    entry_length: usize,
    entry_count: usize,
    entries_address: usize,
    encoder: Encoder
}

#[derive(Debug)]
pub struct Word {
    pub text: String,
    pub position: usize
}

impl Dictionary {
    pub fn new(mem: &MemoryMap) -> Result<Dictionary,InfocomError> {
        let address = mem.get_word(0x08)? as usize;
        let decoder = Decoder::new(mem)?;
        let encoder = Encoder::new(mem)?;

        let mut separators:HashSet<char> = HashSet::new();
        let separator_count = mem.get_byte(address)? as usize;
        for i in 0..separator_count {
            separators.insert(decoder.zscii_to_char(mem.get_byte(address + 1 + i)? as u16)?);
        }

        let entry_length = mem.get_byte(address + 1 + separator_count)? as usize;
        let entry_count = mem.get_word(address + 2 + separator_count)? as usize;
        let entries_address = address + 4 + separator_count;
        
        Ok(Dictionary { address, separators, entry_length, entry_count, entries_address, encoder })
    }

    fn lookup_word(&self, mem: &MemoryMap, word: &str) -> Result<Option<u16>,InfocomError> {
        // TODO: Version 5 support
        let encoded_text = self.encoder.encode(word)?;
        let entry = ((encoded_text[0] as u64) << 16) | encoded_text[1] as u64;

        debug!("{:?} -> ${:012x}", encoded_text, entry);

        // TODO: Binary search this mother.InfocomError
        for i in 0..self.entry_count {
            let entry_address = self.entries_address + (i * self.entry_length);
            let e = ((mem.get_word(entry_address)? as u64) << 16) |
                    mem.get_word(entry_address + 2)? as u64;
            if entry == e {
                return Ok(Some(entry_address as u16));
            }                    
        }

        Ok(None)
    }
        
    pub fn analyze_text(&self, f: &mut FrameStack, text: &String, parse_table_address: usize) -> Result<(),InfocomError> {
        let mut slice = text.as_str();
        let mut words:Vec<Word> = Vec::new();
        let mut offset = 0;
        loop {
            if let Some(i) = slice.find(|c| c == ' ' || self.separators.contains(&c)) {
                if i > 0 {
                    words.push(Word { text: String::from(&slice[0..i]), position: offset });
                    if self.separators.contains(&slice.chars().collect::<Vec<char>>()[i]) {
                        words.push(Word { text: String::from(&slice[i..i+1]), position: offset + i })
                    }
                }
                offset += i + 1;
                slice = &slice[i+1..];
            } else {
                if slice.len() > 0 {
                    words.push(Word { text: String::from(&slice[0..]), position: offset });
                } 
             
                break;
            }
        }

        f.set_byte(parse_table_address + 1, words.len() as u8)?;

        for i in 0..words.len() {
            let addr = parse_table_address + 2 + (4 * i);
            if let Some(entry_address) = self.lookup_word(f.get_memory(), &words[i].text)? {
                debug!("Found {} @ ${:04x}", words[i].text, entry_address);
                f.set_word(addr, entry_address)?;
            } else {
                debug!("{} not in dictionary", words[i].text);
                f.set_word(addr, 0)?;
            }
            f.set_byte(addr + 2, words[i].text.len() as u8)?;
            f.set_byte(addr + 3, words[i].position as u8 + 2)?;
        }

        Ok(())
    }
}
