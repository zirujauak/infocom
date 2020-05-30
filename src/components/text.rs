use std::convert::TryInto;
use std::char;
use log::{debug, error};

use super::InfocomError;
use super::memory::{MemoryMap, Version};

struct Alphabet {
    alphabet: [[char; 26]; 3],
    zscii_table: Vec<char>
}

impl Alphabet {
    pub fn new (mem: &MemoryMap) -> Result<Alphabet,InfocomError> {
        let mut zscii_table = vec!['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß', '»', '«', 'ë', 'ï', 'ÿ', 'Ë', 'Ï', 'á', 'é',
                                   'í', 'ó', 'ú', 'ý', 'Á', 'É', 'Í', 'Ó', 'Ú', 'Ý', 'à', 'è', 'ì', 'ò', 'ù', 'À',
                                   'È', 'Ì', 'Ò', 'Ù', 'â', 'ê', 'î', 'ô', 'û', 'Â', 'Ê', 'Ô', 'Û', 'å', 'Å', 'ø',
                                   'Ø', 'ã', 'ñ', 'õ', 'Ã', 'Ñ', 'Õ', 'æ', 'Æ', 'ç', 'Ç', 'þ', 'ð', 'Þ', 'Ð', '£',
                                   'œ', 'Œ', '¡', '¿'];
        let m = mem.get_memory();
        match mem.version {
            Version::V(1) => Ok(Alphabet { zscii_table,
                                           alphabet: [['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
                                                       'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z'],
                                                      ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M',
                                                       'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z'],
                                                      [' ', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', ',',
                                                       '!', '?', '_', '#', '\'', '"', '/', '\\', '<', '-', ':', '(', ')']]}),
            Version::V(2) | Version::V(3) | Version::V(4) => Ok(Alphabet { zscii_table,
                                                                           alphabet: [['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
                                                                                       'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z'],
                                                                                      ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M',
                                                                                       'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z'],
                                                                                      [' ', '\n', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', 
                                                                                       ',', '!', '?', '_', '#', '\'', '"', '/', '\\', '-', ':', '(', ')']]}),
            Version::V(5) | Version::V(6) | Version::V(7) | Version::V(8) => {
                let alphabet_addr:usize = read_word(&m, 0x34) as usize;
                let alphabet = if alphabet_addr == 0 {
                    [['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
                      'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z'],
                     ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M',
                      'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z'],
                     [' ', '\n', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', 
                      ',', '!', '?', '_', '#', '\'', '"', '/', '\\', '-', ':', '(', ')']]
                } else {
                    let mut alphabet:[[char; 26]; 3] = [[' '; 26]; 3];
                    for i in 0..3 {
                        for j in 0..26 {
                            let addr:usize = alphabet_addr + (i * 26) + j;
                            alphabet[i][j] = m[addr] as char;
                        }
                    }
                    alphabet[2][0] = ' ';
                    alphabet[2][1] = '\n';
                    alphabet
                };

                // Untested: get unicode translation table from header extension table and convert UTF-16 values to UTF-8 
                let extension_table_addr = mem.get_word(0x36)? as usize;
                if extension_table_addr != 0  {
                    let entries = mem.get_word(extension_table_addr)?;
                    if entries >= 3  {
                        let zscii_table_address = mem.get_word(extension_table_addr + 6)? as usize;
                        if zscii_table_address != 0  {
                            let count = mem.get_byte(zscii_table_address)?;
                            let mut utf16:Vec<u16> = Vec::new();
                            for i in 0 .. count {
                                utf16.push(mem.get_word(zscii_table_address + 1 + (2 * i as usize))?);
                            }
                            zscii_table = char::decode_utf16(utf16.iter().cloned()).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
                                                    .collect::<Vec<_>>();
                        }
                    }
                }

                Ok(Alphabet { zscii_table,
                              alphabet })
            },
            _ => Err(InfocomError::Version(mem.version))
        }
    }
}

pub struct Decoder {
    memory: Vec<u8>,
    version: Version,
    alphabet: Alphabet
}

impl Decoder {
    pub fn new(mem: &MemoryMap) -> Result<Decoder,InfocomError> {
        let alphabet = Alphabet::new(mem)?;
        Ok(Decoder { memory: mem.get_memory(), version: mem.version, alphabet })
    }

    pub fn decode(&self, address: usize) -> Result<String, InfocomError> {
        match self.version {
            Version::V(1) => DecoderV1::decode(&self.memory, &self.alphabet, address, true),
            Version::V(2) => DecoderV2::decode(&self.memory, &self.alphabet, address, true),
            _ => DecoderV3::decode(&self.memory, &self.alphabet, address, true)
        }
    }
   
    pub fn zscii_to_char(&self, z: u16) -> Result<char,InfocomError> {
        if z > 1023 {
            return Err(InfocomError::Text(format!("Invalid character code ${:04x}", z)))
        } else {
            match z {
                0 => Ok('\0'),
                13 => Ok('\n'),
                32..=126 => Ok(z as u8 as char),
                _ => if z >= 155 && z < 155 + self.alphabet.zscii_table.len() as u16{
                    Ok(self.alphabet.zscii_table[z as usize - 155])
                } else {
                    Ok('@')
                }
            }  
        }
    }
}


fn read_word(map: &Vec<u8>, address: usize) -> u16 {
    let high = map[address];
    let low = map[address + 1];

    (((high as u16) << 8) & 0xFF00) | ((low as u16) & 0xFF)
}

fn read_zbytes(map: &Vec<u8>, address: usize) -> Vec<u8> {
    let mut b:Vec<u8> = Vec::new();
    let mut i = address;
    loop {
        let v = read_word(map, i);
        let b1:u8 = ((&v >> 10) & 0x1F) as u8;
        let b2:u8 = ((&v >> 5) & 0x1F) as u8;
        let b3:u8 = (&v & 0x1F) as u8;
        b.push(b1);
        b.push(b2);
        b.push(b3);
        if (v & 0x8000) == 0x8000 {
            return b;
        }
        i = i + 2;
    }
}

fn shift(a: usize, direction: isize) -> usize {
    let new_a:isize = a as isize + direction;
    match new_a {
        -1 => 2,
        3 => 0,
        _ => new_a.try_into().unwrap()
    }
}

fn decode_zscii(alphabet: &Alphabet, b1: u8, b2: u8) -> char {
    let z:usize = (((b1 as usize) << 5) & 0x3E) | ((b2 as usize) & 0x1F);
    match z {
        0 => '\0',
        13 => '\n',
        32..=126 => z as u8 as char,
        _ => if z >= 155 && z <= 155 + alphabet.zscii_table.len() {
            alphabet.zscii_table[z - 155]
        } else {
            '@'
        }
    }
}

fn abbreviation_address(map: &Vec<u8>, table: usize, index: usize) -> usize {
    let table_address:usize = read_word(map, 0x18).into();
    let entry_address = table_address + (64 * (table - 1)) + (2 * index);
    read_word(map, entry_address) as usize * 2
}

trait TextDecoder {
    fn decode(map: &Vec<u8>, alphabet: &Alphabet, address: usize, with_abbreviations: bool) -> Result<String,InfocomError>;
}

struct DecoderV1;
struct DecoderV2;
struct DecoderV3; 

impl TextDecoder for DecoderV1 {
    fn decode(map: &Vec<u8>, alphabet: &Alphabet, address: usize, _with_abbreviations: bool) -> Result<String, InfocomError> {
        let data:Vec<u8> = read_zbytes(map, address);
        let mut string = String::new();
        let mut a:usize = 0;
        let mut current_a:usize = 0;
        let mut i = data.iter();

        loop {
            if let Some(c) = i.next() {
                match c {
                    0 => string.push(' '),
                    1 => string.push('\n'),
                    2 => { current_a = shift(a, 1); continue }
                    3 => { current_a = shift(a, -1); continue }
                    4 => a = shift(a, 1),
                    5 => a = shift(a, -1),
                    6 => if a == 2 {
                        if let Some(b1) = i.next() {
                            if let Some(b2) = i.next() {
                                string.push(decode_zscii(alphabet, *b1, *b2))
                            } else {
                                error!("Text ended on incomplete ZSCII character: ${:06x}", address);
                                return Err(InfocomError::Text(format!("Text ended on an incomplete ZSCII character")))
                            }
                        } else {
                            error!("Text ended on incomplete ZSCII character: ${:06x}", address);
                            return Err(InfocomError::Text(format!("Text ended on an incomplete ZSCII character")))
                        }
                    } else {
                        string.push(alphabet.alphabet[current_a][(*c as usize) - 6])
                    }
                    _ => string.push(alphabet.alphabet[current_a][(*c as usize) - 6])
                }
                current_a = a;
            } else {
                return Ok(string)
            }
        }
    }
}

impl TextDecoder for DecoderV2 {
    fn decode(map: &Vec<u8>, alphabet: &Alphabet, address: usize, with_abbreviations: bool) -> Result<String, InfocomError> {
        let data:Vec<u8> = read_zbytes(map, address);
        let mut string = String::new();
        let mut a:usize = 0;
        let mut current_a:usize = 0;
        let mut i = data.iter();

        loop {
            if let Some(c) = i.next() {
                match c {
                    0 => string.push(' '),
                    1 => {
                        if with_abbreviations {
                            if let Some(abbrev) = i.next() {
                                let abbrev_addr = abbreviation_address(map, *c as usize, *abbrev as usize);
                                match DecoderV2::decode(map, alphabet, abbrev_addr, false) {
                                    Ok(s) => string.push_str(&s),
                                    Err(e) => return Err(e)
                                }
                            } else {
                                error!("Text ended on incomplete abbreviation: ${:06x}", address);
                                return Err(InfocomError::Text(format!("Text ended on an incomplete abbreviation")))
                            }
                        } else {
                            error!("Nested abbreviations not allowed: ${:06x}", address);
                            return Err(InfocomError::Text(format!("Nested abbreviations not allowed")))
                        }
                    },
                    2 => { current_a = shift(a, 1); continue }
                    3 => { current_a = shift(a, -1); continue }
                    4 => a = shift(a, 1),
                    5 => a = shift(a, -1),
                    6 => if a == 2 {
                        if let Some(b1) = i.next() {
                            if let Some(b2) = i.next() {
                                string.push(decode_zscii(alphabet, *b1, *b2))
                            } else {
                                error!("Text ended on incomplete ZSCII character: ${:06x}", address);
                                return Err(InfocomError::Text(format!("Text ended on an incomplete ZSCII character")))
                            }
                        } else {
                            error!("Text ended on incomplete ZSCII character: ${:06x}", address);
                            return Err(InfocomError::Text(format!("Text ended on an incomplete ZSCII character")))
                        }
                    } else {
                        string.push(alphabet.alphabet[current_a][(*c as usize) - 6])
                    }
                    _ => string.push(alphabet.alphabet[current_a][(*c as usize) - 6])
                }
                current_a = a;
            } else {
                return Ok(string)
            }
        }
    }
}

impl TextDecoder for DecoderV3 {
    fn decode(map: &Vec<u8>, alphabet: &Alphabet, address: usize, with_abbreviations: bool) -> Result<String, InfocomError> {
        let data:Vec<u8> = read_zbytes(map, address);
        let mut string = String::new();
        let mut a:usize = 0;
        let mut i = data.iter();

        loop {
            if let Some(c) = i.next() {
                match c {
                    0 => string.push(' '),
                    1 | 2 | 3  => {
                        if with_abbreviations {
                            if let Some(abbrev) = i.next() {
                                let abbrev_addr = abbreviation_address(map, *c as usize, *abbrev as usize);
                                match DecoderV3::decode(map, alphabet, abbrev_addr, false) {
                                    Ok(s) => string.push_str(&s),
                                    Err(e) => return Err(e)
                                }
                            } else {
                                error!("Text ended on incomplete abbreviation: ${:06x}", address);
                                return Err(InfocomError::Text(format!("Text ended on an incomplete abbreviation")))
                            }
                        } else {
                            error!("Nested abbreviations not allowed: ${:06x}", address);
                            return Err(InfocomError::Text(format!("Nested abbreviations not allowed")))
                        }
                    },
                    4 => a = 1,
                    5 => a = 2,
                    6 => if a == 2 {
                        if let Some(b1) = i.next() {
                            if let Some(b2) = i.next() {
                                string.push(decode_zscii(alphabet, *b1, *b2))
                            } else {
                                error!("Text ended on incomplete ZSCII character: ${:06x}", address);
                                return Err(InfocomError::Text(format!("Text ended on an incomplete ZSCII character")))
                            }
                        } else {
                            error!("Text ended on incomplete ZSCII character: ${:06x}", address);
                            return Err(InfocomError::Text(format!("Text ended on an incomplete ZSCII character")))
                        }
                    } else {
                        string.push(alphabet.alphabet[a][(*c as usize) - 6])
                    }
                    7 => if a == 2 {
                        string.push('\n')
                    } else {
                        string.push(alphabet.alphabet[a][(*c as usize) - 6])
                    }
                    _ => string.push(alphabet.alphabet[a][(*c as usize) - 6])
                }
                if *c != 4 && *c != 5 {
                    a = 0;
                }
            } else {
                return Ok(string)
            }
        }
    }
}

pub struct Encoder {
    version: Version,
    alphabet: Alphabet,
}

impl Encoder {
    pub fn new(mem: &MemoryMap) -> Result<Encoder,InfocomError> {
        let alphabet = Alphabet::new(mem)?;
        Ok(Encoder { version: mem.version,
                     alphabet })
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u16>, InfocomError> {
        let s = String::from(text).to_lowercase();
        match self.version {
            Version::V(1) | Version::V(2) => self.encode_text(&s, 6, true),
            Version::V(3) => self.encode_text(&s, 6, false),
            _ => self.encode_text(&s, 9, false)
        }
    }

    fn encode_text(&self, text: &str, length: usize, shift_lock: bool) -> Result<Vec<u16>, InfocomError> {
        let zchars:Vec<u8> = self.to_zchars(text, length, shift_lock);
        let mut result:Vec<u16> = Vec::new();

        for i in (0..length).step_by(3) {
            let zc1 = zchars.get(i).unwrap();
            let zc2 = zchars.get(i+1).unwrap();
            let zc3 = zchars.get(i+2).unwrap();

            let zb1 = ((zc1 << 2) & 0x7C) | ((zc2 >> 3) & 0x03);
            let zb2 = ((zc2 << 5) & 0xE0) | zc3;
            result.push((((zb1 as u16) << 8) & 0xFF00) | (zb2 as u16 & 0xFF));
        }

        let last = result.pop().unwrap();
        result.push(last | 0x8000);
        Ok(result)
    }

    fn map_char(&self, c: char) -> Option<(u8, u8)> {
        for i in 0..3 {
            for j in 0..self.alphabet.alphabet[i].len() {
                if c == self.alphabet.alphabet[i][j] {
                    return Some((i as u8, j as u8 + 6));
                }
            }
        }

        debug!("{} {}", c, c as u8);
        for i in 0..self.alphabet.zscii_table.len() {
            if c == *self.alphabet.zscii_table.get(i).unwrap() {
                debug!("ZSCII: {}", 155 + i);
                let b1 = ((155 + i) >> 5) & 0x1F;
                let b2 = (155 + i) & 0x1F;
                return Some((b1 as u8 | 0x80, b2 as u8));
            }
        }

        None
    }

    pub fn to_bytes(&self, text: &str) -> Vec<u8> {
        let mut result:Vec<u8> = Vec::new();

        for c in text.chars() {
            if (c as u16) > 31 && (c as u16) < 127 {
                result.push(c as u8);
                continue;
            }

            // TODO: Map extended characters
            for (i, z) in self.alphabet.zscii_table.iter().enumerate() {
                if *z == c {
                    result.push(155 as u8 + i as u8)
                }
            }
        }

        result
    }

    fn to_zchars(&self, text: &str, length: usize, shift_lock: bool) -> Vec<u8> {
        let mut result:Vec<u8> = Vec::new();
        let mut iterator = text.chars().peekable();
        let mut shift_locked = false;

        while result.len() < length {
            if let Some(c) = iterator.next() {
                if let Some((a, i)) = self.map_char(c) {
                    // High bit of the alphabet byte set means this is a 10-bit ZSCII character code
                    if a & 0x80 == 0x80 {
                        result.push(5);
                        result.push(6);
                        result.push(a & 0x1F);
                        result.push(i);
                    } else {
                        if shift_locked && a != 2 {
                        result.push(i);
                        shift_locked = false;
                        result.push(4);
                        } else if a == 2 {
                            // If no shift-locking, push a shift
                            if !shift_lock {
                                result.push(5);
                            } else if !shift_locked {
                                // Peek at the next character and if it's also A2, push a shift-lock
                                if let Some(n_c) = iterator.peek() {
                                    if let Some((n_a, _)) = self.map_char(*n_c as char) {
                                        if n_a == 2 {
                                            shift_locked = true;
                                            result.push(5);
                                        }
                                    } else {
                                        result.push(3);
                                    }
                                } else {
                                    result.push(3);
                                }
                            }
                        }

                        result.push(i as u8);
                    }
                }
            } else {
                break;
            }
        }

        result.append(&mut vec![5; length]);
        result.truncate(length);
        result
    }
}