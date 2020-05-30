extern crate easycurses;

use std::collections::HashSet;
use easycurses::*;
use easycurses::Color::*;

use log::debug;

pub enum StatusLineFormat {
    SCORED,
    TIMED
}

pub trait Interface {
    fn print(&mut self, text: &str);
    fn new_line(&mut self);
    fn read(&mut self, terminating_characters: HashSet<char>, max_chars: usize) -> String;
    fn status_line(&mut self, name: &str, format: StatusLineFormat, v1: i16, v2: u16);
}

pub struct Curses {
    pub window: EasyCurses
}

impl Curses {
    pub fn new() -> Curses {
        let mut window = EasyCurses::initialize_system().unwrap();
        window.resize(40,132);
        debug!("{:?}", window.set_scrolling(true));
        debug!("{:?}", window.set_scroll_region(1, 39));
        window.move_rc(39, 0);
        window.set_echo(false);
        window.set_input_mode(easycurses::InputMode::RawCharacter);
        window.refresh();
        window.set_color_pair(colorpair!(White on Black));

        Curses { window: window }
    }
}

impl Interface for Curses {
    fn print(&mut self, text: &str) {
        let words: Vec<&str> = text.split(' ').collect();
        debug!("{:?}", words);
        let (rows, cols) = self.window.get_row_col_count();
        debug!("{} {}", rows, cols);
        for (i, word) in words.iter().enumerate() {
            let (r,c) = self.window.get_cursor_rc();
            debug!("{},{} => {} :: {}", r, c, word.len(), cols - c);
            if word.len() > cols as usize - c as usize {
                self.window.print_char('\n');
                // if r == rows - 1 {
                //     self.window.move_rc(0, 0);
                //     self.window.delete_line();
                //     self.window.move_rc(rows - 1, 0);
                // } else {
                //     self.window.move_rc(r + 1 , 0);
                // }
            }
            self.window.print(word);
            if i < words.len() - 1 {
                self.window.print_char(' ');
            }
        }
        
        self.window.refresh();
    }

    fn new_line(&mut self) {
        self.window.print_char('\n');
        self.window.refresh();
    }

    fn read(&mut self, terminating_characters: HashSet<char>, max_chars: usize) -> String {
        let mut result = String::new();
        loop {
            if let Some(e) = self.window.get_input() {
                let (r,c) = self.window.get_cursor_rc();
                debug!("get_input() -> {:?} at {},{}", e, r, c);
                match e {
                    easycurses::Input::Character(c) => {
                        if terminating_characters.contains(&c) {
                            result.push(c);
                            self.new_line();
                            break;
                        }

                        if c as u16 == 8 {
                            if result.len() > 0 {
                                result.pop();
                                let (r,c) = self.window.get_cursor_rc();
                                self.window.move_rc(r, c - 1);
                                self.window.delete_char();
                                self.window.refresh();
                            }
                        // TODO: Filter the specific accented characters that we support
                        // TODO: include A2 punctuation
                        } else if c.is_alphabetic() || c.is_ascii() || c as u16 == 32 {
                            if result.len() < max_chars {
                                self.window.print_char(c);
                                self.window.refresh();
                                result.push(c);
                            }
                        }
                    },
                    easycurses::Input::KeyEnter => {
                        result.push('\n');
                        break;
                    },
                    _ => {}
                }
            }
        }
        
        result
    }

    fn status_line(&mut self, name: &str, format: StatusLineFormat, v1: i16, v2: u16) {
        let (r,c) = self.window.get_cursor_rc();
        let width = self.window.get_row_col_count().1;

        self.window.move_rc(0, 0);
        self.window.set_color_pair(colorpair!(Black on White));
        
        self.window.print_char(' ');
        self.window.print(name);

        let left_str = match format {
            StatusLineFormat::SCORED => {
                format!("Score: {:3}    Turn: {:4} ", v1, v2)
            },
            StatusLineFormat::TIMED => {
                let hour = v1.rem_euclid(12);
                let am_pm = if v1 > 11 { "PM" } else { "AM" };
                format!("{:2}:{:02} {} ", hour, v2, am_pm)
            }
        };

        let padding = width as usize - 1 - name.len() - left_str.len();
        for i in 0..padding {
            self.window.print_char(' ');
        }
        self.window.print(left_str);

        self.window.set_color_pair(colorpair!(White on Black));
        self.window.move_rc(r, c);
        self.window.refresh();
    }
}