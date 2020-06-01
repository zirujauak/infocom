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
    pub window: EasyCurses,
    printed_lines: u32,
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

        Curses { window: window, printed_lines: 0 }
    }

    fn prompt(&mut self) {
        let (rows,_) = self.window.get_row_col_count();
        let (r,_) = self.window.get_cursor_rc();

        if self.printed_lines as i32 >= rows - 2 {
            self.window.move_rc(r, 0);
            self.window.print("[MORE]");
            self.window.refresh();
            self.window.get_input();
            self.window.move_rc(r, 0);
            self.window.print("      ");
            self.window.move_rc(r, 0);
            self.window.refresh();
            self.printed_lines = 0;
        }
    }
}

impl Interface for Curses {    
    fn print(&mut self, text: &str) {
        let words: Vec<&str> = text.split(' ').collect();
        let (_, cols) = self.window.get_row_col_count();
        for (i, word) in words.iter().enumerate() {
            let (_,c) = self.window.get_cursor_rc();
            if i > 0 && c < cols - 1 {
                self.window.print_char(' ');
            }

            // Check if the word contains any newline characters
            let mut slice = *word;
            while let Some(j) = slice.find('\n') {
                let (_,c) = self.window.get_cursor_rc();
                let part = &slice[0..j];
                // Check if the slice is too long to fit in the remaining space
                if part.len() + 1 >= cols as usize - c as usize {
                    self.new_line();
                }

                // Print the part before the new line
                self.window.print(part);
                // ... and the new line
                self.new_line();

                // Check if there's more after the newline
                if j < slice.len() {
                    slice = &slice[j+1..];
                } else {
                    slice = "";
                }
            }
            
            // If there's any text left over, print it, too
            if slice.len() > 0 {
                let (_,c) = self.window.get_cursor_rc();
                if slice.len() >= cols as usize - c as usize {
                    self.new_line();
                }

                self.window.print(slice);
            }
        }

        self.window.refresh();
    }

    fn new_line(&mut self) {
        let (rows,_) = self.window.get_row_col_count();
        let (r, _) = self.window.get_cursor_rc();

        if r == rows - 1 {
            self.window.move_rc(1,0);
            self.window.delete_line();
            self.window.move_rc(r, 0);
        } else {
            self.window.move_rc(r + 1, 0);
        }
        self.printed_lines += 1;
        self.window.refresh();
        self.prompt();

    }

    fn read(&mut self, terminating_characters: HashSet<char>, max_chars: usize) -> String {
        self.printed_lines = 0;
        let mut result = String::new();
        loop {
            if let Some(e) = self.window.get_input() {
                let (r,c) = self.window.get_cursor_rc();
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
                        } else if ((c as u16) > 31 && (c as u16) < 127) || ((c as u16) > 155 && (c as u16) < 256) {
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
        for _ in 0..padding {
            self.window.print_char(' ');
        }
        self.window.print(left_str);

        self.window.set_color_pair(colorpair!(White on Black));
        self.window.move_rc(r, c);
        self.window.refresh();
    }
}