use std::env;
use std::error::Error; // ‼️ This line is no longer needed
use std::fs;
use std::io::{Result, Write, stdout}; // ‼️ Added `Result` here
use std::time::Duration;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    queue,
    style, // ‼️ Added `style` for `Print` command
    terminal::{self, ClearType},
    // ‼️ Removed `Result,` as it's not in crossterm's root
};

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
    // ‼️ Removed `_original_terminal_state` field.
    // ‼️ `enable_raw_mode` doesn't return a struct to store anymore.
    // ‼️ We just call `disable_raw_mode` in the Drop impl.
}

impl Editor {
    /// Creates a new Editor instance, loading a file from the command line arguments.
    fn new() -> Result<Self> {
        // ‼️ This now refers to `std::io::Result`
        let (cols, rows) = terminal::size()?;
        terminal::enable_raw_mode()?; // ‼️ Just call this, don't store the result
        let mut editor = Self {
            cx: 0,
            cy: 0,
            screen_rows: rows as usize - 1, // Reserve one line for status bar
            screen_cols: cols as usize,
            rows: Vec::new(),
            row_offset: 0,
            status_msg: "HELP: :q = quit".to_string(),
            // ‼️ Removed `_original_terminal_state` from initialization
        };

        // Try to load a file from the first command-line argument
        if let Some(filename) = env::args().nth(1) {
            editor.load_file(&filename);
        } else {
            // Start with an empty buffer if no file is provided
            editor.rows.push(String::new());
        }

        Ok(editor)
    }

    /// Loads the content of a file into the editor's `rows` buffer.
    fn load_file(&mut self, filename: &str) {
        match fs::read_to_string(filename) {
            Ok(content) => {
                self.rows = content.lines().map(|s| s.to_string()).collect();
                self.status_msg = format!("Loaded file: {}", filename);
            }
            Err(e) => {
                self.rows.push(String::new()); // Start with empty buffer on error
                self.status_msg = format!("Failed to load file: {}", e);
            }
        }
    }

    /// The main event loop, waiting for input and processing it.
    fn run(&mut self) -> Result<()> {
        // ‼️ Changed return type
        self.refresh_screen()?;

        loop {
            // Poll for an event with a timeout. This makes the editor feel responsive.
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key_event) = event::read()? {
                    if self.process_keypress(key_event)? == false {
                        return Ok(()); // ‼️ Exit requested, return Ok
                    }
                }
            }
            // In a more advanced editor, you might handle other events here,
            // like terminal resize, mouse clicks, etc.

            self.refresh_screen()?;
        }
    }

    /// Handles a single keypress event.
    /// Returns `Ok(false)` if the user quits, `Ok(true)` otherwise.
    fn process_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        match event.code {
            // For now, we only have "Normal" mode.
            // A ':' would eventually switch to "Command" mode.
            // An 'i' would eventually switch to "Insert" mode.

            // --- MOVEMENT ---
            KeyCode::Char('h') | KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                // Allow moving one past the end of the line, like Vim
                let current_line_len = self
                    .rows
                    .get(self.cy + self.row_offset)
                    .map_or(0, |line| line.len());
                if self.cx < current_line_len {
                    self.cx += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                // ‼️ This logic is updated to handle scrolling
                if self.cy > 0 {
                    self.cy -= 1;
                } else if self.row_offset > 0 {
                    // ‼️ We're at the top of the screen, scroll up
                    self.row_offset -= 1;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                // ‼️ This logic is updated to handle scrolling
                let file_last_row = self.rows.len().saturating_sub(1);
                if self.cy + self.row_offset < file_last_row {
                    // ‼️ We are not on the last line of the file
                    if self.cy < self.screen_rows - 1 {
                        // ‼️ We are not on the last line of the screen
                        self.cy += 1;
                    } else {
                        // ‼️ We are on the last line of the screen, scroll
                        self.row_offset += 1;
                    }
                }
            }

            // --- SCROLLING (half-page) ---
            KeyCode::Char('d') if event.modifiers == event::KeyModifiers::CONTROL => {
                // Ctrl-D (Page Down)
                let new_offset =
                    (self.row_offset + self.screen_rows / 2).min(self.rows.len().saturating_sub(1));
                let dy = new_offset - self.row_offset;
                self.row_offset = new_offset;
                self.cy = self.cy.saturating_sub(dy); // Try to keep cursor stationary relative to text
                self.scroll_check();
            }
            KeyCode::Char('u') if event.modifiers == event::KeyModifiers::CONTROL => {
                // Ctrl-U (Page Up)
                let new_offset = self.row_offset.saturating_sub(self.screen_rows / 2);
                let dy = self.row_offset - new_offset;
                self.row_offset = new_offset;
                self.cy = (self.cy + dy).min(self.screen_rows - 1); // Try to keep cursor stationary
                self.scroll_check();
            }

            // --- QUITTING ---
            // This is a placeholder. A real Vim would use Command mode.
            KeyCode::Char(':') => {
                // A temporary, simple command mode
                if self.prompt_command() == false {
                    return Ok(false); // Quit
                }
            }

            _ => {
                // Not implemented yet (like 'i' for insert)
            }
        }

        // After every move, clamp cursor to end of line
        self.clamp_cursor_to_line();
        self.scroll_check();
        Ok(true)
    }

    /// A very simple command-line prompter.
    /// Returns false if the user quits, true otherwise.
    fn prompt_command(&mut self) -> bool {
        let mut stdout = stdout();
        let mut command = String::from(":");
        loop {
            // Draw the command at the bottom
            queue!(
                stdout,
                cursor::MoveTo(0, self.screen_rows as u16),
                terminal::Clear(ClearType::CurrentLine),
            )
            .unwrap();
            print!("{}", command);
            stdout.flush().unwrap();

            if let Ok(Event::Key(key_event)) = event::read() {
                match key_event.code {
                    KeyCode::Enter => {
                        if command == ":q" {
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

    /// A very simple command-line prompter.

    /// Clears the screen and redraws all content.
    fn refresh_screen(&mut self) -> Result<()> {
        // ‼️ This now refers to `std::io::Result`
        let mut stdout = stdout();
        // `queue!` is faster than `execute!` because it batches all
        // terminal commands and sends them at once on `flush`.
        queue!(
            stdout,
            // Hide cursor, clear screen, move to top-left
            cursor::Hide,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0),
        )?;

        self.draw_rows()?;
        self.draw_status_bar()?;

        // Move the cursor to its correct position
        queue!(
            stdout,
            cursor::MoveTo(self.cx as u16, self.cy as u16),
            cursor::Show
        )?;

        stdout.flush()
    }

    /// Ensures the cursor is within the visible screen area, adjusting scroll if needed.
    fn scroll_check(&mut self) {
        // ‼️ This block clamps the cursor to the file size after a page jump
        if self.cy + self.row_offset >= self.rows.len() {
            if self.rows.len() > 0 {
                self.cy = self
                    .rows
                    .len()
                    .saturating_sub(1)
                    .saturating_sub(self.row_offset);
            } else {
                self.cy = 0; // ‼️ Handle empty file
            }
        }

        // ‼️ Removed the "Scroll up" block that caused the compile error.
        // ‼️ `self.cy` is usize and can't be < 0.

        // ‼️ Removed the "Scroll down" block.
        // ‼️ The new 'j' key logic handles this scrolling incrementally.

        // ‼️ Make sure row_offset doesn't go too far (e.g., after Ctrl-D)
        self.row_offset = self.row_offset.min(self.rows.len().saturating_sub(1));
    }

    /// Ensures the horizontal cursor (cx) isn't past the end of the current line.
    fn clamp_cursor_to_line(&mut self) {
        let current_line_len = self
            .rows
            .get(self.cy + self.row_offset)
            .map_or(0, |line| line.len());

        // In Vim, you can place the cursor one *past* the end of the line
        // but not in an empty line.
        if self.cx > current_line_len {
            self.cx = current_line_len;
        }
        if current_line_len == 0 && self.cx > 0 {
            self.cx = 0;
        }
    }

    /// Draws the text buffer to the screen.
    fn draw_rows(&self) -> Result<()> {
        // ‼️ This now refers to `std::io::Result`
        let mut stdout = stdout();
        for y in 0..self.screen_rows {
            let file_row_index = y + self.row_offset;
            if file_row_index >= self.rows.len() {
                // We're past the end of the file, draw welcome lines
                if self.rows.len() == 1 && self.rows[0].is_empty() && y == self.screen_rows / 3 {
                    let welcome = "Vim-like Editor - v0.0.1";
                    let padding = (self.screen_cols.saturating_sub(welcome.len())) / 2;
                    let padding_str = " ".repeat(padding);
                    // ‼️ `print!` returns `()` which doesn't implement `Command`.
                    // ‼️ We must use `style::Print` inside `queue!`.
                    queue!(
                        stdout,
                        cursor::MoveTo(0, y as u16),
                        style::Print(format!("~{}{}", padding_str, welcome))
                    )?;
                } else {
                    // ‼️ Use `style::Print`
                    queue!(stdout, cursor::MoveTo(0, y as u16), style::Print("~"))?;
                }
            } else {
                // Draw a line from the file
                let line = &self.rows[file_row_index];
                let len = line.len().min(self.screen_cols);
                // ‼️ Use `style::Print`
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
        // ‼️ This now refers to `std::io::Result`
        let mut stdout = stdout();
        queue!(
            stdout,
            cursor::MoveTo(0, self.screen_rows as u16),
            // Invert colors for status bar
            crossterm::style::SetBackgroundColor(crossterm::style::Color::DarkGrey),
            crossterm::style::SetForegroundColor(crossterm::style::Color::Black)
        )?;

        // Build status text
        let file_row = self.cy + self.row_offset + 1;
        let total_rows = self.rows.len();
        let status = format!(
            "{}  -- {}:{} -- {}/{}",
            self.status_msg,
            self.cx + 1,
            file_row,
            file_row,
            total_rows
        );
        let status_len = status.len().min(self.screen_cols);

        print!("{}", &status[..status_len]);

        // Fill rest of the line
        print!(
            "{}",
            " ".repeat(self.screen_cols.saturating_sub(status_len))
        );

        // Reset colors
        queue!(stdout, crossterm::style::ResetColor)?;
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
            cursor::MoveTo(0, 0)
        )
        .ok();
    }
}

/// Main function: setup and error handling.
fn main() -> Result<()> {
    // ‼️ This now refers to `std::io::Result`
    // We use a block here so that `editor` is dropped *before*
    // we try to print the error, ensuring the terminal is reset.
    let run_result = {
        let mut editor = Editor::new()?;
        editor.run() // ‼️ This now returns Result<(), io::Error>
    };

    if let Err(e) = run_result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}
