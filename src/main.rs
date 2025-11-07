use crossterm::{
    cursor::{self, SetCursorStyle},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue, style,
    terminal::{self, ClearType},
};
use ropey::Rope;
use std::env;
use std::fs;
use std::io::{Result, Write, stdout};
use std::time::Duration;

struct Buffer {
    rope: Rope,
    filename: Option<String>,
    dirty: bool,
}

impl Buffer {
    /// Creates a new, empty buffer.
    fn new() -> Self {
        Self {
            rope: Rope::new(),
            filename: None,
            dirty: false,
        }
    }
    /// Creates a buffer by loading a file.
    fn from_file(filename: &str) -> Result<Self> {
        match fs::File::open(filename) {
            Ok(file) => {
                let rope = Rope::from_reader(std::io::BufReader::new(file))?;
                Ok(Self {
                    rope,
                    filename: Some(filename.to_string()),
                    dirty: false,
                })
            }
            Err(e) => {
                // If file doesn't exist, create an empty buffer with that name
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(Self {
                        rope: Rope::new(),
                        filename: Some(filename.to_string()),
                        dirty: false,
                    })
                } else {
                    Err(e)
                }
            }
        }
    }
    /// Saves the buffer to its filename.
    fn save(&mut self) -> Result<bool> {
        if let Some(filename) = &self.filename {
            let file = fs::File::create(filename)?;
            self.rope.write_to(std::io::BufWriter::new(file))?;
            self.dirty = false;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// Returns the number of lines in the buffer.
    fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }
    /// Returns a slice of a specific line.
    fn line(&self, index: usize) -> ropey::RopeSlice<'_> {
        self.rope.line(index)
    }
    /// Inserts a character at (line, col).
    fn insert_char(&mut self, line: usize, col: usize, c: char) {
        let line_char_idx = self.rope.line_to_char(line);
        self.rope.insert_char(line_char_idx + col, c);
        self.dirty = true;
    }
    /// Deletes a character at (line, col) [for Backspace].
    fn delete_char(&mut self, line: usize, col: usize) {
        if col > 0 {
            let line_char_idx = self.rope.line_to_char(line);
            self.rope
                .remove((line_char_idx + col - 1)..(line_char_idx + col));
            self.dirty = true;
        }
    }
    /// Inserts a newline at (line, col).
    fn insert_new_line(&mut self, line: usize, col: usize) {
        let char_idx = self.rope.line_to_char(line) + col;
        self.rope.insert(char_idx, "\n"); // ‼️ Just insert newline text
        self.dirty = true;
    }
    /// Joins the given line with the previous one [for Backspace at col 0].
    /// Returns the new `cx` (length of the previous line).
    fn join_with_previous_line(&mut self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        // Get length of previous line *before* joining
        let prev_line_len = self.rope.line(line - 1).len_chars();
        // Find the char index of the newline to remove
        let prev_line_end_char = self.rope.line_to_char(line);
        self.rope
            .remove((prev_line_end_char - 1)..prev_line_end_char);
        self.dirty = true;
        prev_line_len
    }
}
struct NormalState {}
struct InsertState {}
struct VisualState {
    // Needs to store the origin point of the selection
    selection_start: (usize, usize),
}
struct CommandState {
    // Needs to store the text buffer for the command line
    command_buffer: String,
}
// 2. Define the main Mode enum
enum Mode {
    Normal(NormalState),
    Insert(InsertState),
    Visual(VisualState),
    Command(CommandState),
}
// End new states
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
    /// The file buffer
    buffer: Buffer,
    /// The row index of the file that is at the top of the screen (for scrolling).
    row_offset: usize,
    /// A message to display in the status bar.
    status_msg: String,
    mode: Mode,
}
impl Editor {
    /// Creates a new Editor instance, loading a file from the command line arguments.
    fn new() -> Result<Self> {
        let (cols, rows) = terminal::size()?;
        terminal::enable_raw_mode()?;
        // ‼️ Load buffer based on args
        let buffer = if let Some(filename) = env::args().nth(1) {
            Buffer::from_file(&filename)?
        } else {
            Buffer::new()
        };
        let mut editor = Self {
            cx: 0,
            cy: 0,
            screen_rows: rows as usize - 1,
            screen_cols: cols as usize,
            buffer,
            row_offset: 0,
            status_msg: "HELP: :q = quit".to_string(),
            mode: Mode::Normal(NormalState {}),
        };
        // Set status message from buffer loading
        if editor.buffer.filename.is_some() {
            editor.status_msg = format!(
                "Loaded file: {}",
                editor.buffer.filename.as_deref().unwrap()
            );
        } else if editor.buffer.rope.len_chars() > 0 {
            // Loaded from stdin or other source, but no filename
            editor.status_msg = "[No Name]".to_string();
        }
        Ok(editor)
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
            Mode::Normal(_) => self.process_normal_keypress(event), // ‼️
            Mode::Insert(_) => self.process_insert_keypress(event), // ‼️
            Mode::Visual(_) => self.process_visual_keypress(event), // ‼️
            Mode::Command(_) => self.process_command_keypress(event), // ‼️
        }
    }
    // --- Normal Mode Logic ---
    /// Handles key events in Normal mode.
    fn process_normal_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        // Clear status message on most keypresses
        if !matches!(event.code, KeyCode::Char(':')) {
            self.status_msg.clear();
        }
        match event.code {
            // --- MOVEMENT ---
            KeyCode::Char('h') | KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                // Get current line length from buffer
                let file_row = self.cy + self.row_offset;
                let current_line_len = if file_row < self.buffer.len_lines() {
                    self.buffer.line(file_row).len_chars() // ‼️ Get char length
                } else {
                    0
                };
                // In normal mode, cursor can't go past the last char (if line not empty)
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
                // Get file length from buffer
                let file_last_row = self.buffer.len_lines().saturating_sub(1);
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
                let new_offset = (self.row_offset + self.screen_rows / 2)
                    .min(self.buffer.len_lines().saturating_sub(1)); // ‼️ Use buffer.len_lines()
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
                self.mode = Mode::Insert(InsertState {});
                self.status_msg = "-- INSERT --".to_string();
            }
            KeyCode::Char('v') => {
                self.mode = Mode::Visual(VisualState {
                    selection_start: (self.cx, self.cy + self.row_offset),
                });
                self.status_msg = "-- VISUAL --".to_string();
            }
            // --- COMMANDS ---
            KeyCode::Char(':') => {
                self.mode = Mode::Command(CommandState {
                    command_buffer: ":".to_string(),
                });
                self.status_msg.clear();
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
        self.status_msg.clear(); // Clear status message on any insert mode keypress
        match event.code {
            // --- MODE SWITCHING ---
            KeyCode::Esc => {
                self.mode = Mode::Normal(NormalState {});
                self.status_msg.clear();
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
    // --- Visual Mode Logic ---
    /// Handles key events in Visual mode.
    fn process_visual_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        self.status_msg.clear();
        match event.code {
            // --- MODE SWITCHING ---
            KeyCode::Esc => {
                self.mode = Mode::Normal(NormalState {});
                self.status_msg.clear();
            }
            // TODO: Add visual mode movement and commands (y, d, etc.)
            // For now, just movement like normal mode
            KeyCode::Char('h') | KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                // Get current line length from buffer
                let file_row = self.cy + self.row_offset;
                let current_line_len = if file_row < self.buffer.len_lines() {
                    self.buffer.line(file_row).len_chars()
                } else {
                    0
                };
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
                // ‼️ Get file length from buffer
                let file_last_row = self.buffer.len_lines().saturating_sub(1);
                if self.cy + self.row_offset < file_last_row {
                    if self.cy < self.screen_rows - 1 {
                        self.cy += 1;
                    } else {
                        self.row_offset += 1;
                    }
                }
            }
            _ => {}
        }
        self.clamp_cursor_to_line();
        self.scroll_check();
        Ok(true)
    }
    // --- Command Mode Logic ---
    /// Handles key events in Command mode.
    fn process_command_keypress(&mut self, event: KeyEvent) -> Result<bool> {
        let Mode::Command(state) = &mut self.mode else {
            return Ok(true); // Should not happen
        };
        match event.code {
            KeyCode::Enter => {
                let command_to_execute = state.command_buffer.clone();
                // Switch back to Normal mode *before* executing
                self.mode = Mode::Normal(NormalState {});
                self.execute_command(&command_to_execute)
            }
            KeyCode::Esc => {
                self.mode = Mode::Normal(NormalState {}); // Switch to Normal
                self.status_msg.clear();
                Ok(true)
            }
            KeyCode::Char(c) => {
                state.command_buffer.push(c);
                Ok(true)
            }
            KeyCode::Backspace => {
                if state.command_buffer.len() > 1 {
                    state.command_buffer.pop();
                } else {
                    // ‼️ Popped the ':', abort to Normal mode
                    self.mode = Mode::Normal(NormalState {});
                    self.status_msg.clear();
                }
                Ok(true)
            }
            _ => Ok(true),
        }
    }
    /// Executes a command string.
    fn execute_command(&mut self, command: &str) -> Result<bool> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(true); // Should not happen
        }
        match parts[0] {
            ":q" => {
                if self.buffer.dirty {
                    self.status_msg =
                        "No write since last change (use :q! to override)".to_string();
                    Ok(true) // Don't quit
                } else {
                    Ok(false) // Quit
                }
            }
            ":q!" => {
                Ok(false) // Force quit
            }
            ":w" => {
                if parts.len() > 1 {
                    // Update buffer's filename
                    self.buffer.filename = Some(parts[1].to_string());
                }
                // Tell buffer to save
                if self.buffer.save()? {
                    self.status_msg =
                        format!("Saved file: {}", self.buffer.filename.as_deref().unwrap());
                } else {
                    self.status_msg = "No filename specified. Use :w <filename>".to_string();
                }
                Ok(true) // Continue
            }
            ":wq" => {
                // Tell buffer to save
                let save_success = if parts.len() > 1 {
                    self.buffer.filename = Some(parts[1].to_string());
                    self.buffer.save()?
                } else {
                    self.buffer.save()?
                };
                // Only quit if save was successful or file wasn't dirty
                if save_success {
                    self.status_msg =
                        format!("Saved file: {}", self.buffer.filename.as_deref().unwrap());
                    Ok(false) // Quit
                } else if !self.buffer.dirty {
                    Ok(false) // Quit (wasn't dirty)
                } else {
                    // Save failed (e.g., no filename)
                    self.status_msg = "No filename specified. Use :w <filename>".to_string();
                    Ok(true) // Don't quit
                }
            }
            _ => {
                self.status_msg = format!("Unknown command: {}", command);
                Ok(true) // Continue
            }
        }
    }
    /// Inserts a character at the cursor position.
    fn insert_char(&mut self, c: char) {
        let file_row = self.cy + self.row_offset;
        // Get line length from buffer
        let line_len = if file_row < self.buffer.len_lines() {
            self.buffer.line(file_row).len_chars()
        } else {
            0
        };
        if self.cx > line_len {
            self.cx = line_len;
        }
        // Call buffer's insert method
        self.buffer.insert_char(file_row, self.cx, c);
        self.cx += 1;
    }
    /// Inserts a new line at the cursor position.
    fn insert_new_line(&mut self) {
        let file_row = self.cy + self.row_offset;
        // Get line length from buffer
        let line_len = if file_row < self.buffer.len_lines() {
            self.buffer.line(file_row).len_chars()
        } else {
            0
        };
        if self.cx > line_len {
            self.cx = line_len;
        }
        // Call buffer's new line method
        self.buffer.insert_new_line(file_row, self.cx);
        // Move cursor
        self.cx = 0;
        if self.cy < self.screen_rows - 1 {
            self.cy += 1;
        } else {
            self.row_offset += 1;
        }
    }
    /// Deletes a character at the cursor position (Backspace).
    fn delete_char(&mut self) {
        let file_row = self.cy + self.row_offset;
        if self.cx == 0 {
            // At the start of a line, join with the previous line
            if file_row > 0 {
                // Tell buffer to join lines
                let prev_line_len = self.buffer.join_with_previous_line(file_row);
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
            // Get line length from buffer
            let line_len = if file_row < self.buffer.len_lines() {
                self.buffer.line(file_row).len_chars()
            } else {
                0
            };
            if self.cx > line_len {
                self.cx = line_len;
            }
            if self.cx > 0 {
                //  Tell buffer to delete
                self.buffer.delete_char(file_row, self.cx);
                self.cx -= 1;
            }
        }
    }

    // --- Visual Mode Helper ---
    /// Gets the normalized selection range (start_pos, end_pos).
    /// Start_pos is always <= end_pos.
    fn get_selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        if let Mode::Visual(state) = &self.mode {
            let start_pos = state.selection_start; // (x, y_file)
            let end_pos = (self.cx, self.cy + self.row_offset); // (x, y_file)

            if end_pos.1 < start_pos.1 || (end_pos.1 == start_pos.1 && end_pos.0 < start_pos.0) {
                Some((end_pos, start_pos)) // Swap if end is before start
            } else {
                Some((start_pos, end_pos))
            }
        } else {
            None
        }
    }

    /// Clears the screen and redraws all content.
    fn refresh_screen(&mut self) -> Result<()> {
        let mut stdout = stdout();
        // ‼️ Set cursor style based on mode
        match self.mode {
            Mode::Normal(_) => queue!(stdout, SetCursorStyle::SteadyBlock)?,
            Mode::Insert(_) => queue!(stdout, SetCursorStyle::SteadyBar)?,
            Mode::Visual(_) => queue!(stdout, SetCursorStyle::SteadyBlock)?,
            Mode::Command(_) => queue!(stdout, SetCursorStyle::SteadyBar)?,
        }
        queue!(
            stdout,
            cursor::Hide,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0),
        )?;
        self.draw_rows()?;
        self.draw_status_bar()?;
        // ‼️ Move cursor to correct position based on mode
        let (cx, cy) = if let Mode::Command(state) = &self.mode {
            // ‼️ In command mode, cursor is on status line
            let cx = state.command_buffer.len().min(self.screen_cols - 1);
            let cy = self.screen_rows;
            (cx, cy)
        } else {
            // In other modes, cursor is in the text area
            (self.cx, self.cy)
        };
        queue!(stdout, cursor::MoveTo(cx as u16, cy as u16), cursor::Show)?;
        stdout.flush()
    }
    /// Ensures the cursor is within the visible screen area, adjusting scroll if needed.
    fn scroll_check(&mut self) {
        if self.cy + self.row_offset >= self.buffer.len_lines() {
            if self.buffer.len_lines() > 0 {
                self.cy = self
                    .buffer
                    .len_lines()
                    .saturating_sub(1)
                    .saturating_sub(self.row_offset);
            } else {
                self.cy = 0;
            }
        }
        self.row_offset = self
            .row_offset
            .min(self.buffer.len_lines().saturating_sub(1));
    }
    /// Ensures the horizontal cursor (cx) isn't past the end of the current line.
    fn clamp_cursor_to_line(&mut self) {
        let file_row = self.cy + self.row_offset;
        // Get line length from buffer
        let current_line_len = if file_row < self.buffer.len_lines() {
            self.buffer.line(file_row).len_chars()
        } else {
            0
        };
        match self.mode {
            // Clamp for Normal and Visual
            Mode::Normal(_) | Mode::Visual(_) => {
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
            // Clamp for Insert
            Mode::Insert(_) => {
                // In Insert mode, cursor can go one *past* the last char
                if self.cx > current_line_len {
                    self.cx = current_line_len;
                }
            }
            // Do nothing for command mode
            Mode::Command(_) => {}
        }
        if current_line_len == 0 && self.cx > 0 {
            self.cx = 0;
        }
    }
    /// Draws the text buffer to the screen.
    fn draw_rows(&self) -> Result<()> {
        let mut stdout = stdout();

        // Get the selection range *once* before the loop
        let selection = self.get_selection_range();

        for y in 0..self.screen_rows {
            let file_row_index = y + self.row_offset;
            queue!(stdout, cursor::MoveTo(0, y as u16))?; // Move cursor at start of loop

            if file_row_index >= self.buffer.len_lines() {
                // Welcome message logic
                if self.buffer.len_lines() == 1
                    && self.buffer.line(0).len_chars() == 0
                    && y == self.screen_rows / 3
                {
                    let welcome = "Vim-like Editor - v0.0.1";
                    let padding = (self.screen_cols.saturating_sub(welcome.len())) / 2;
                    let padding_str = " ".repeat(padding);
                    queue!(
                        stdout,
                        // No need for MoveTo, already at (0, y)
                        style::Print(format!("~{}{}", padding_str, welcome))
                    )?;
                } else {
                    queue!(stdout, style::Print("~"))?;
                }
            } else {
                // Get line from buffer
                let line = self.buffer.line(file_row_index);
                let start_char = 0; // ‼️ TODO: Add horizontal scrolling
                let end_char = (start_char + self.screen_cols).min(line.len_chars());

                // --- Highlighting Logic ---
                let mut is_highlighted = false;

                // Iterate over the chars we are actually drawing
                for (cx, char) in line.chars().enumerate().skip(start_char).take(end_char) {
                    let mut should_highlight = false;

                    // Check if this char (cx, file_row_index) is in selection
                    if let Some(((start_x, start_y), (end_x, end_y))) = selection {
                        if file_row_index > start_y && file_row_index < end_y {
                            should_highlight = true;
                        } else if file_row_index == start_y && file_row_index == end_y {
                            should_highlight = cx >= start_x && cx <= end_x;
                        } else if file_row_index == start_y {
                            should_highlight = cx >= start_x;
                        } else if file_row_index == end_y {
                            should_highlight = cx <= end_x;
                        }
                    }

                    // Apply/remove highlighting if state changes
                    if should_highlight && !is_highlighted {
                        queue!(stdout, style::SetAttribute(style::Attribute::Reverse))?;
                        is_highlighted = true;
                    } else if !should_highlight && is_highlighted {
                        queue!(stdout, style::ResetColor)?;
                        is_highlighted = false;
                    }

                    queue!(stdout, style::Print(char))?;
                }

                if is_highlighted {
                    queue!(stdout, style::ResetColor)?;
                }
                // --- End Highlighting Logic ---
            }
            // Clear rest of the line (in case new line is shorter than old)
            queue!(stdout, terminal::Clear(ClearType::UntilNewLine))?;
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
        // Build status text based on mode
        let (mode_str, status_to_show) = match &self.mode {
            Mode::Normal(_) => ("-- NORMAL --", self.status_msg.clone()),
            Mode::Insert(_) => ("-- INSERT --", self.status_msg.clone()),
            Mode::Visual(_) => ("-- VISUAL --", self.status_msg.clone()), // ‼️
            Mode::Command(state) => ("", state.command_buffer.clone()),   // ‼️
        };
        let file_row = self.cy + self.row_offset + 1;
        // Get total rows from buffer
        let total_rows = self.buffer.len_lines();
        let right_status = format!(
            "{}:{} -- {}/{}",
            self.cx + 1,
            file_row,
            file_row,
            total_rows
        );
        // Build left status string
        let left_status = if !status_to_show.is_empty() {
            status_to_show
        } else {
            // Show mode, filename, and dirty status
            let filename_str = self.buffer.filename.as_deref().unwrap_or("[No Name]");
            let dirty_str = if self.buffer.dirty { " [+]" } else { "" };
            format!("{} \"{}\"{}", mode_str, filename_str, dirty_str)
        };
        let right_len = right_status.len();
        let left_len = left_status
            .len()
            .min(self.screen_cols.saturating_sub(right_len + 1));
        // Ensure left_status is not truncated mid-char
        let (left_status_truncated, left_len) = if left_status.len() > left_len {
            let mut new_len = left_len;
            while !left_status.is_char_boundary(new_len) {
                new_len -= 1;
            }
            (&left_status[..new_len], new_len)
        } else {
            (left_status.as_str(), left_len)
        };
        let padding = " ".repeat(self.screen_cols.saturating_sub(left_len + right_len));
        // Use style::Print for status bar content
        queue!(
            stdout,
            style::Print(left_status_truncated),
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
