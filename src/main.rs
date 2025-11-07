use std::env;
use std::fs;
use std::io::{Result, Write, stdout};
use std::time::Duration;

use crossterm::{
    cursor::{self, SetCursorStyle},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue, style,
    terminal::{self, ClearType},
};

enum Mode {
    Normal,
    Insert,
}

/// Represents the editor's state.
struct Editor {
    /// The user's horizontal cursor position.
    cx: usize,
    /// The user's vertical cursor position.
    cy: usize,
    /// The number of rows in the terminal.
    screen_rows: usize,
    /// The number of columns in the terminal.
    screen_cols: usize,
    /// The text content, as a vector of strings (one per line).
    rows: Vec<String>,
    /// The row index of the file that is at the top of the screen (for scrolling).
    row_offset: usize,
    /// A message to display in the status bar.
    status_msg: String,
    mode: Mode,
    /// The name of the file being edited
    filename: Option<String>,
}

impl Editor {
    /// Creates a new Editor instance, loading a file from the command line arguments.
    fn new() -> Result<Self> {
        let (cols, rows) = terminal::size()?;
        terminal::enable_raw_mode()?;
        let mut editor = Self {
            cx: 0,
            cy: 0,
            screen_rows: rows as usize - 1,
            screen_cols: cols as usize,
            rows: Vec::new(),
            row_offset: 0,
            status_msg: "HELP: :q = quit".to_string(),
            mode: Mode::Normal,
            filename: None,
        };

        if let Some(filename) = env::args().nth(1) {
            editor.filename = Some(filename.clone());
            editor.load_file(&filename);
        } else {
            editor.rows.push(String::new());
        }

        Ok(editor)
    }

    /// Loads the content of a file into the editor's `rows` buffer.
    fn load_file(&mut self, filename: &str) {
        match fs::read_to_string(filename) {
            Ok(content) => {
                self.rows = content.lines().map(|s| s.to_string()).collect();
                if self.rows.is_empty() {
                    self.rows.push(String::new());
                }
                self.status_msg = format!("Loaded file: {}", filename);
            }
            Err(e) => {
                self.rows.push(String::new());
                self.status_msg = format!("Failed to load file: {}", e);
            }
        }
    }

    /// The main event loop, waiting for input and processing it.
    fn run(&mut self) -> Result<()> {
        self.refresh_screen()?;

        loop {
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key_event) = event::read()? {
                    // ‼️ process_keypress now routes to other functions
                    if self.process_keypress(key_event)? == false {
                        return Ok(());
                    }
                }
            }

            self.refresh_screen()?;
        }
    }

    // --- Main Keypress Router ---
    /// Routes key events to the correct handler based on the current mode.
    fn process_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        match self.mode {
            Mode::Normal => self.process_normal_keypress(event),
            Mode::Insert => self.process_insert_keypress(event),
        }
    }

    // --- Normal Mode Logic ---
    /// Handles key events in Normal mode.
    fn process_normal_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        match event.code {
            // --- MOVEMENT ---
            KeyCode::Char('h') | KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let current_line_len = self
                    .rows
                    .get(self.cy + self.row_offset)
                    .map_or(0, |line| line.len());
                // ‼️ In normal mode, cursor can't go past the last char (if line not empty)
                let max_cx = if current_line_len > 0 {
                    current_line_len - 1
                } else {
                    0
                };
                if self.cx < max_cx {
                    self.cx += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.cy > 0 {
                    self.cy -= 1;
                } else if self.row_offset > 0 {
                    self.row_offset -= 1;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let file_last_row = self.rows.len().saturating_sub(1);
                if self.cy + self.row_offset < file_last_row {
                    if self.cy < self.screen_rows - 1 {
                        self.cy += 1;
                    } else {
                        self.row_offset += 1;
                    }
                }
            }

            // --- SCROLLING (half-page) ---
            KeyCode::Char('d') if event.modifiers == KeyModifiers::CONTROL => {
                let new_offset =
                    (self.row_offset + self.screen_rows / 2).min(self.rows.len().saturating_sub(1));
                let dy = new_offset - self.row_offset;
                self.row_offset = new_offset;
                self.cy = self.cy.saturating_sub(dy);
                self.scroll_check();
            }
            KeyCode::Char('u') if event.modifiers == KeyModifiers::CONTROL => {
                let new_offset = self.row_offset.saturating_sub(self.screen_rows / 2);
                let dy = self.row_offset - new_offset;
                self.row_offset = new_offset;
                self.cy = (self.cy + dy).min(self.screen_rows - 1);
                self.scroll_check();
            }

            // --- MODE SWITCHING ---
            KeyCode::Char('i') => {
                self.mode = Mode::Insert;
                self.status_msg = "-- INSERT --".to_string();
            }

            // --- COMMANDS ---
            KeyCode::Char(':') => {
                if self.prompt_command() == false {
                    return Ok(false); // Quit
                }
            }

            _ => {}
        }

        self.clamp_cursor_to_line();
        self.scroll_check();
        Ok(true)
    }

    //  --- Insert Mode Logic ---
    /// Handles key events in Insert mode.
    fn process_insert_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        match event.code {
            // --- MODE SWITCHING ---
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status_msg = "".to_string();
                self.clamp_cursor_to_line();
            }

            // --- TYPING ---
            KeyCode::Char(c) => {
                self.insert_char(c);
            }

            // --- ENTER ---
            KeyCode::Enter => {
                self.insert_new_line();
            }

            // --- BACKSPACE ---
            KeyCode::Backspace => {
                self.delete_char();
            }

            _ => {}
        }
        Ok(true)
    }

    /// Inserts a character at the cursor position.
    fn insert_char(&mut self, c: char) {
        let file_row = self.cy + self.row_offset;
        if let Some(line) = self.rows.get_mut(file_row) {
            // ‼️ Ensure cursor isn't past the end of the line
            let line_len = line.len();
            if self.cx > line_len {
                self.cx = line_len;
            }
            line.insert(self.cx, c);
            self.cx += 1;
        }
    }

    /// Inserts a new line at the cursor position.
    fn insert_new_line(&mut self) {
        let file_row = self.cy + self.row_offset;
        if let Some(line) = self.rows.get_mut(file_row) {
            let line_len = line.len();
            if self.cx > line_len {
                self.cx = line_len;
            }

            // Split the current line at the cursor
            let new_line = line.split_off(self.cx);
            self.rows.insert(file_row + 1, new_line);

            // Move cursor
            self.cx = 0;
            if self.cy < self.screen_rows - 1 {
                self.cy += 1;
            } else {
                self.row_offset += 1;
            }
        }
    }

    /// Deletes a character at the cursor position (Backspace).
    fn delete_char(&mut self) {
        let file_row = self.cy + self.row_offset;

        if self.cx == 0 {
            // At the start of a line, join with the previous line
            if file_row > 0 {
                let prev_line = self.rows.remove(file_row);
                let prev_line_len = self.rows[file_row - 1].len();
                self.rows[file_row - 1].push_str(&prev_line);

                // Move cursor
                if self.cy > 0 {
                    self.cy -= 1;
                } else {
                    self.row_offset -= 1;
                }
                self.cx = prev_line_len;
            }
        } else {
            // In the middle of a line, remove the character to the left
            if let Some(line) = self.rows.get_mut(file_row) {
                let line_len = line.len();
                if self.cx > line_len {
                    self.cx = line_len;
                }

                if self.cx > 0 {
                    line.remove(self.cx - 1);
                    self.cx -= 1;
                }
            }
        }
    }

    /// A very simple command-line prompter.
    fn prompt_command(&mut self) -> bool {
        let mut stdout = stdout();
        let mut command = String::from(":");
        loop {
            // Draw the command at the bottom
            queue!(
                stdout,
                cursor::MoveTo(0, self.screen_rows as u16),
                terminal::Clear(ClearType::CurrentLine),
                // ‼️ Use style::Print for command prompt
                style::Print(&command)
            )
            .unwrap();
            stdout.flush().unwrap();

            if let Ok(Event::Key(key_event)) = event::read() {
                match key_event.code {
                    KeyCode::Enter => {
                        if command == ":q" {
                            return false; // User wants to quit
                        }
                        if command == ":w" {
                            self.save_file();
                            return true;
                        }
                        if command == ":wq" {
                            self.save_file();
                            return false; // User wants to quit
                        }
                        self.status_msg = format!("Unknown command: {}", command);
                        return true;
                    }
                    KeyCode::Esc => {
                        self.status_msg = "".to_string();
                        return true; // Abort command
                    }
                    KeyCode::Char(c) => {
                        command.push(c);
                    }
                    KeyCode::Backspace => {
                        command.pop();
                        if command.is_empty() {
                            self.status_msg = "".to_string();
                            return true; // Aborted by deleting ':'
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Saves the current buffer to the file.
    fn save_file(&mut self) {
        if let Some(filename) = &self.filename {
            match fs::write(filename, self.rows.join("\n")) {
                Ok(_) => {
                    self.status_msg = format!("Saved file: {}", filename);
                }
                Err(e) => {
                    self.status_msg = format!("Error saving file: {}", e);
                }
            }
        } else {
            // TODO: Implement "Save As" logic in prompt_command
            self.status_msg = "No filename specified. Use :w <filename>".to_string();
        }
    }

    /// Clears the screen and redraws all content.
    fn refresh_screen(&mut self) -> Result<()> {
        let mut stdout = stdout();

        // Set cursor style based on mode
        match self.mode {
            Mode::Normal => queue!(stdout, SetCursorStyle::SteadyBlock)?,
            Mode::Insert => queue!(stdout, SetCursorStyle::SteadyBar)?,
        }

        queue!(
            stdout,
            cursor::Hide,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0),
        )?;

        self.draw_rows()?;
        self.draw_status_bar()?;

        // In insert mode, cursor can be one past the line
        let cx = self.cx;
        let cy = self.cy;

        queue!(stdout, cursor::MoveTo(cx as u16, cy as u16), cursor::Show)?;

        stdout.flush()
    }

    /// Ensures the cursor is within the visible screen area, adjusting scroll if needed.
    fn scroll_check(&mut self) {
        if self.cy + self.row_offset >= self.rows.len() {
            if self.rows.len() > 0 {
                self.cy = self
                    .rows
                    .len()
                    .saturating_sub(1)
                    .saturating_sub(self.row_offset);
            } else {
                self.cy = 0;
            }
        }

        self.row_offset = self.row_offset.min(self.rows.len().saturating_sub(1));
    }

    /// Ensures the horizontal cursor (cx) isn't past the end of the current line.
    fn clamp_cursor_to_line(&mut self) {
        let file_row = self.cy + self.row_offset;
        let current_line_len = self.rows.get(file_row).map_or(0, |line| line.len());

        match self.mode {
            Mode::Normal => {
                // In Normal mode, cursor stays *on* the last char
                let max_cx = if current_line_len > 0 {
                    current_line_len - 1
                } else {
                    0
                };
                if self.cx > max_cx {
                    self.cx = max_cx;
                }
            }
            Mode::Insert => {
                // In Insert mode, cursor can go one *past* the last char
                if self.cx > current_line_len {
                    self.cx = current_line_len;
                }
            }
        }

        if current_line_len == 0 && self.cx > 0 {
            self.cx = 0;
        }
    }

    /// Draws the text buffer to the screen.
    fn draw_rows(&self) -> Result<()> {
        let mut stdout = stdout();
        for y in 0..self.screen_rows {
            let file_row_index = y + self.row_offset;
            if file_row_index >= self.rows.len() {
                if self.rows.len() == 1 && self.rows[0].is_empty() && y == self.screen_rows / 3 {
                    let welcome = "Vim-like Editor - v0.0.1";
                    let padding = (self.screen_cols.saturating_sub(welcome.len())) / 2;
                    let padding_str = " ".repeat(padding);
                    queue!(
                        stdout,
                        cursor::MoveTo(0, y as u16),
                        style::Print(format!("~{}{}", padding_str, welcome))
                    )?;
                } else {
                    queue!(stdout, cursor::MoveTo(0, y as u16), style::Print("~"))?;
                }
            } else {
                let line = &self.rows[file_row_index];
                let len = line.len().min(self.screen_cols);
                queue!(
                    stdout,
                    cursor::MoveTo(0, y as u16),
                    style::Print(&line[..len])
                )?;
            }
        }
        Ok(())
    }

    /// Draws the status bar at the bottom of the screen.
    fn draw_status_bar(&self) -> Result<()> {
        let mut stdout = stdout();
        queue!(
            stdout,
            cursor::MoveTo(0, self.screen_rows as u16),
            style::SetBackgroundColor(style::Color::DarkGrey),
            style::SetForegroundColor(style::Color::Black)
        )?;

        // Build status text with Mode
        let mode_str = match self.mode {
            Mode::Normal => "-- NORMAL --",
            Mode::Insert => "-- INSERT --",
        };
        let file_row = self.cy + self.row_offset + 1;
        let total_rows = self.rows.len();

        // Show status message if it exists, otherwise show mode
        let left_status = if !self.status_msg.is_empty() {
            &self.status_msg
        } else {
            mode_str
        };

        let right_status = format!(
            "{}:{} -- {}/{}",
            self.cx + 1,
            file_row,
            file_row,
            total_rows
        );
        let right_len = right_status.len();
        let left_len = left_status
            .len()
            .min(self.screen_cols.saturating_sub(right_len + 1));

        let padding = " ".repeat(self.screen_cols.saturating_sub(left_len + right_len));

        // Use style::Print for status bar content
        queue!(
            stdout,
            style::Print(left_status),
            style::Print(padding),
            style::Print(right_status),
            style::ResetColor
        )?;

        Ok(())
    }
}

/// Disables raw mode when the Editor is dropped (e.g., on panic or exit).
impl Drop for Editor {
    fn drop(&mut self) {
        terminal::disable_raw_mode().ok();
        execute!(
            stdout(),
            cursor::Show,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0),
            SetCursorStyle::DefaultUserShape
        )
        .ok();
    }
}

/// Main function: setup and error handling.
fn main() -> Result<()> {
    let run_result = {
        let mut editor = Editor::new()?;
        editor.run()
    };

    if let Err(e) = run_result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}
