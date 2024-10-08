/* * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * *\
Filename : lib.rs

Copyright (C) 2021 CJ McAllister
    This program is free software; you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation; either version 3 of the License, or
    (at your option) any later version.
    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.
    You should have received a copy of the GNU General Public License
    along with this program; if not, write to the Free Software Foundation,
    Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301  USA

Purpose:
    This library provides a multi-threaded, global logger.

    All logging actions occur in the logging thread, leaving the main thread
    free to do all the cool stuff it wants to do!

\* * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * */

#![warn(missing_docs)]
#![cfg_attr(not(doctest), doc = include_str!("../README.md"))]

use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvError, SendError};
use std::sync::Arc;
use std::thread;

use chrono::DateTime;
use chrono::Local;
use once_cell::sync::OnceCell;


///////////////////////////////////////////////////////////////////////////////
//  Named Constants
///////////////////////////////////////////////////////////////////////////////

// Buffer size of the sync_channel for sending log messages
const CHANNEL_SIZE: usize = 512;


///////////////////////////////////////////////////////////////////////////////
//  Module Declarations
///////////////////////////////////////////////////////////////////////////////

#[doc(hidden)]
pub mod sender;
use self::sender::Sender;

#[doc(hidden)]
pub mod receiver;
use self::receiver::Receiver;


///////////////////////////////////////////////////////////////////////////////
//  Data Structures
///////////////////////////////////////////////////////////////////////////////

/// Denotes the level or severity of the log message.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub enum Level {
    /// For tracing code paths. Mean to be very verbose.
    Trace = 0x01,
    /// For debugging and troubleshooting
    Debug = 0x02,
    /// For harmless but useful information
    Info = 0x04,
    /// For cosmetic/recoverable errors
    Warning = 0x08,
    /// For Major/unrecoverable errors
    Error = 0x10,
    /// For Very Bad News™
    Fatal = 0x20,
}

#[doc(hidden)]
/// Tuple struct containing log message and its log level
pub struct MsgTuple {
    pub timestamp: DateTime<Local>,
    pub level: Level,
    pub fn_name: String,
    pub line: u32,
    pub msg: String,
}

/// Specifies which stream(s) log messages should be written to.
#[derive(Debug, Copy, Clone)]
pub enum OutputStream {
    /// Don't write to either stream, i.e., disable logging
    Neither = 0x0,
    /// Write only to StdOut
    StdOut = 0x1,
    /// Write only to a file
    File = 0x2,
    /// Write to both StdOut and a File
    Both = 0x3,
}

#[doc(hidden)]
/// Enumeration of commands that the logging thread will handle
pub enum Command {
    LogMsg(MsgTuple),
    SetOutputLevel(Level),
    SetOutputStream(OutputStream),
    Flush(mpsc::Sender<()>),
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct MtLogger {
    enabled: bool,
    sender: Sender,
    msg_count: Arc<AtomicU64>,
}

/// Logging errors
#[derive(Debug)]
pub enum MtLoggerError {
    /// A logging command was attempted before the global logger instance was initialized with [`mt_new!`]
    LoggerNotInitialized,

    /* Wrappers */
    /// Wrapper for `SendError<>`
    SendError(SendError<Command>),
    /// Wrapper for `RecvError`
    RecvError(RecvError),
}

#[doc(hidden)]
pub static INSTANCE: OnceCell<MtLogger> = OnceCell::new();


///////////////////////////////////////////////////////////////////////////////
//  Object Implementation
///////////////////////////////////////////////////////////////////////////////

impl MtLogger {
    /// Fully-qualified constructor
    pub fn new(
        logfile_prefix: &'static str,
        output_level: Level,
        output_stream: OutputStream,
    ) -> Self {
        // Create the log messaging and control channel
        // Must be a sync channel in order to wrap OnceCell around an MtLogger
        let (logger_tx, logger_rx) = mpsc::sync_channel::<Command>(CHANNEL_SIZE);

        // Create the shared message count
        let msg_count = Arc::new(AtomicU64::new(0));

        // Initialize receiver struct, build and spawn thread
        let mut log_receiver = Receiver::new(
            logfile_prefix,
            logger_rx,
            output_level,
            output_stream,
            Arc::clone(&msg_count),
        );
        thread::Builder::new()
            .name("log_receiver".to_string())
            .spawn(move || log_receiver.main())
            .unwrap();

        // Initialize sender struct
        let log_sender = Sender::new(logger_tx);

        Self {
            enabled: true,
            sender: log_sender,
            msg_count,
        }
    }


    /*  *  *  *  *  *  *  *\
     *  Accessor Methods  *
    \*  *  *  *  *  *  *  */

    #[doc(hidden)]
    pub fn msg_count(&self) -> u64 {
        self.msg_count.load(Ordering::SeqCst)
    }


    /*  *  *  *  *  *  *  *\
     *   Utility Methods  *
    \*  *  *  *  *  *  *  */

    #[doc(hidden)]
    //FEAT: Bring filtering back to the sending-side
    pub fn log_msg(
        &self,
        timestamp: DateTime<Local>,
        level: Level,
        fn_name: String,
        line: u32,
        msg: String,
    ) -> Result<(), SendError<Command>> {
        // If logging is enabled, package log message into tuple and send
        if self.enabled {
            let log_tuple = MsgTuple {
                timestamp,
                level,
                fn_name,
                line,
                msg,
            };
            self.sender.send_log(Command::LogMsg(log_tuple))
        } else {
            Ok(())
        }
    }

    #[doc(hidden)]
    pub fn log_cmd(&self, cmd: Command) -> Result<(), SendError<Command>> {
        if self.enabled {
            self.sender.send_cmd(cmd)
        } else {
            Ok(())
        }
    }

    #[doc(hidden)]
    pub fn flush(&self) -> Result<(), MtLoggerError> {
        // Create a channel that will be used to notify completion of the flush
        let (flush_ack_tx, flush_ack_rx) = mpsc::channel::<()>();

        // Send a flush command to the receiver thread
        self.sender.send_cmd(Command::Flush(flush_ack_tx))?;

        // Block until the the flush ACK arrives
        flush_ack_rx.recv()?;

        Ok(())
    }
}


///////////////////////////////////////////////////////////////////////////////
//  Static Functions
///////////////////////////////////////////////////////////////////////////////

#[doc(hidden)]
// Wrapper around chrono::Local::now() to avoid dependency issues with using external crate functions in macros
pub fn mt_now() -> DateTime<Local> {
    Local::now()
}


///////////////////////////////////////////////////////////////////////////////
//  Trait Implementations
///////////////////////////////////////////////////////////////////////////////

/*  *  *  *  *  *  *  *\
 *       Level        *
\*  *  *  *  *  *  *  */

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Trace => write!(f, "TRACE"),
            Self::Debug => write!(f, "DEBUG"),
            Self::Info => write!(f, "INFO"),
            Self::Warning => write!(f, "WARNING"),
            Self::Error => write!(f, "ERROR"),
            Self::Fatal => write!(f, "FATAL"),
        }
    }
}


/*  *  *  *  *  *  *  *\
 *    MtLoggerError   *
\*  *  *  *  *  *  *  */

impl Error for MtLoggerError {}

impl fmt::Display for MtLoggerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::LoggerNotInitialized => {
                write!(
                    f,
                    "Attempted a command before the logger instance was initialized"
                )
            }

            // Wrappers
            Self::SendError(send_err) => {
                write!(
                    f,
                    "Encountered SendError '{}' while performing a logger command",
                    send_err
                )
            }
            Self::RecvError(recv_err) => {
                write!(
                    f,
                    "Encountered RecvError '{}' while performing a logger command",
                    recv_err
                )
            }
        }
    }
}

impl From<SendError<Command>> for MtLoggerError {
    fn from(src: SendError<Command>) -> Self {
        Self::SendError(src)
    }
}
impl From<RecvError> for MtLoggerError {
    fn from(src: RecvError) -> Self {
        Self::RecvError(src)
    }
}


///////////////////////////////////////////////////////////////////////////////
//  Macro Definitions
///////////////////////////////////////////////////////////////////////////////

/// Initializes the `mt_logger` global instance.
///
/// # Examples
///
/// Initialize the logger instance to log `Info`-level messages and higher to _both_ StdOut and a file.
/// The filename will be given the default prefix, see module-level documentation for full format.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::{Level, OutputStream};
/// # fn main() {
/// mt_new!(None, Level::Info, OutputStream::Both);
/// # }
/// ```
///
/// Initialize the logger instance to log `Trace`-level messages and higher to a file _only_.
/// The filename will be given the specified prefix.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::{Level, OutputStream};
/// # fn main() {
/// mt_new!(Some("my_app_v2.3"), Level::Trace, OutputStream::File);
/// # }
/// ```
#[macro_export]
macro_rules! mt_new {
    ($logfile_prefix:expr, $output_level:expr, $output_stream:expr) => {{
        // Use prefix if specified, or default to parent package name
        let prefix = match $logfile_prefix {
            Some(specified_prefix) => specified_prefix,
            None => env!("CARGO_PKG_NAME"),
        };

        let logger = $crate::MtLogger::new(prefix, $output_level, $output_stream);

        $crate::INSTANCE
            .set(logger)
            .expect("MtLogger INSTANCE already initialized");
    }};
}

/// Sends a message to be logged at the specified logging level.
///
/// Arguments after `$log_level` follow the format of [`println!`] arguments.
///
/// # Note
/// A call to this macro will only _send_ the message to the logging thread.
/// It does NOT guarantee that the message will be delivered at any time.
///
/// If all messages must be logged at a given time, see [`mt_flush!`].
///
/// # Examples
///
/// Logs a `Debug`-level message with the content, "No response received after 500ms".
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::Level;
/// # fn main() {
/// let timeout = 500;
/// mt_log!(Level::Debug, "No response received after {}ms", timeout);
/// # }
/// ```
#[macro_export]
macro_rules! mt_log {
    ($log_level:expr, $( $fmt_args:expr ),*) => {{
        // Take the timestamp first for highest accuracy
        let timestamp = $crate::mt_now();

        // Capture fully-qualified function name
        let fn_name = {
            fn f() {}
            fn type_name_of<T>(_: T) -> &'static str {
                std::any::type_name::<T>()
            }
            let name = type_name_of(f);
            &name[..name.len() - 3]
        };

        let msg_content: String = format!($( $fmt_args ),*);

        $crate::INSTANCE
            .get()
            // If None is encountered, the logger has not been initialized, so do nothing
            .and_then(|instance| instance.log_msg(
                timestamp,
                $log_level,
                fn_name.to_string(),
                line!(),
                msg_content)
                .ok()
            );
    }};
}

/// Sets the active stream to the specified [`OutputStream`].
///
/// # Examples
///
/// Set active stream to `StdOut`.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::OutputStream;
/// # fn main() {
/// mt_stream!(OutputStream::StdOut);
/// # }
/// ```
///
/// Set active stream to `Neither`, i.e., disable logging.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::OutputStream;
/// # fn main() {
/// mt_stream!(OutputStream::Neither);
/// # }
/// ```
#[macro_export]
macro_rules! mt_stream {
    ($output_stream:expr) => {{
        // Get the global instance and send a command to set the output stream
        $crate::INSTANCE
            .get()
            // If None is encountered, the logger has not been initialized, so do nothing
            .and_then(|instance| {
                instance
                    .log_cmd($crate::Command::SetOutputStream($output_stream))
                    .ok()
            });
    }};
}

/// Sets the minimum logging level to the specified [`Level`].
///
/// # Examples
///
/// Log all messages at `Debug`-level or higher.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::Level;
/// # fn main() {
/// mt_level!(Level::Debug);
/// # }
/// ```
///
/// Log all messages at `Fatal`-level or higher, i.e., only log `Fatal`-level messages.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::Level;
/// # fn main() {
/// mt_level!(Level::Fatal);
/// # }
/// ```
#[macro_export]
macro_rules! mt_level {
    ($output_level:expr) => {{
        // Get the global instance and send a command to set the output level
        $crate::INSTANCE
            .get()
            // If None is encountered, the logger has not been initialized, so do nothing
            .and_then(|instance| {
                instance
                    .log_cmd($crate::Command::SetOutputLevel($output_level))
                    .ok()
            });
    }};
}

/// Returns a count of _recorded_ log messages.
///
/// NOTE: This may not (and likely _is_ not, at any given time), the same as the
/// number of times [`mt_log!`] has been called. The message count is only incremented
/// after the logging thread has successfully written a message to the active
/// stream(s). Due to the nature of multithreading, this may happen at any time or never.
///
/// If a count of all successfully sent and recorded messages is required, [`mt_flush!`]
/// must be called before [`mt_count!`].
///
/// # Examples
///
/// Get count of recorded messages.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::{Level, OutputStream};
/// # fn main() {
/// # mt_new!(None, Level::Info, OutputStream::Both);
/// let msg_count = mt_count!();
/// # }
/// ```
#[macro_export]
macro_rules! mt_count {
    () => {{
        // Get the global instance and retrieve the message count
        $crate::INSTANCE
            .get()
            // If None is encountered, the logger has not been initialized, which is an error
            .and_then(|instance| Some(instance.msg_count()))
            .unwrap()
    }};
}

/// Blocks the calling thread until all messages have been received by the logging thread.
///
/// Returns [`Result<(), MtLoggerError>`]
///
/// # Examples
///
/// Flush all sent messages.
/// ```
/// # #[macro_use] extern crate mt_logger;
/// # use mt_logger::{Level, MtLoggerError, OutputStream};
/// # fn main() -> Result<(), MtLoggerError> {

/// # mt_new!(None, Level::Info, OutputStream::Both);/// mt_log!(Level::Debug, "These");
/// mt_log!(Level::Debug, "messages");
/// mt_log!(Level::Debug, "may");
/// mt_log!(Level::Debug, "not");
/// mt_log!(Level::Debug, "have");
/// mt_log!(Level::Debug, "been");
/// mt_log!(Level::Debug, "received");
/// mt_log!(Level::Debug, "yet.");
///
/// mt_flush!()?;
/// // Now they have!
///
/// Ok(())
/// # }
/// ```
///
/// # Errors
///
/// As this function is effectively a send and blocking receive, it is possible for
/// either of these calls to fail, and those errors will propagate back to the caller.
///
/// See [`MtLoggerError`] for an enumeration of errors that may be returned.
#[macro_export]
macro_rules! mt_flush {
    () => {
        $crate::INSTANCE.get().map_or(
            // If None is encountered, the logger has not been initialized, just return an error
            Err($crate::MtLoggerError::LoggerNotInitialized),
            // If instance is initialized, allow all messages to flush to output
            |instance| instance.flush(),
        )
    };
}


///////////////////////////////////////////////////////////////////////////////
//  Unit Tests
///////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time;

    use lazy_static::lazy_static;

    use regex::Regex;

    use crate::receiver::{FILE_OUT_FILENAME, STDOUT_FILENAME};
    use crate::{Level, OutputStream, INSTANCE};


    type TestResult = Result<(), Box<dyn Error>>;


    lazy_static! {
        static ref LOGGER_MUTEX: Mutex<()> = Mutex::new(());
    }


    #[derive(Debug, PartialEq)]
    enum VerfFile {
        StdOut,
        FileOut,
    }

    const LOGFILE_PREFIX: Option<&'static str> = Some("TEST");

    const STDOUT_HDR_REGEX_STR: &str = r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.\d{9}: \x1b\[(\d{3};\d{3}m)\[(\s*(\w*)\s*)\]\x1b\[0m (.*)\(\) line (\d*):";
    const STDOUT_COLOR_IDX: usize = 1;
    const STDOUT_PADDED_LEVEL_IDX: usize = 2;
    const STDOUT_PADLESS_LEVEL_IDX: usize = 3;
    const STDOUT_FN_NAME_IDX: usize = 4;
    const STDOUT_LINE_NUM_IDX: usize = 5;

    const FILE_OUT_HDR_REGEX_STR: &str =
        r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.\d{9}: \[(\s*(\w*)\s*)\] (.*)\(\) line (\d*):";
    const FILE_OUT_PADDED_LEVEL_IDX: usize = 1;
    const FILE_OUT_PADLESS_LEVEL_IDX: usize = 2;
    const FILE_OUT_FN_NAME_IDX: usize = 3;
    const FILE_OUT_LINE_NUM_IDX: usize = 4;


    // Reset verification files, if they exist
    fn reset_verf_files() -> TestResult {
        let path_buf = PathBuf::from(STDOUT_FILENAME);
        if path_buf.as_path().exists() {
            fs::write(STDOUT_FILENAME, "")?;
        }

        let path_buf = PathBuf::from(FILE_OUT_FILENAME);
        if path_buf.as_path().exists() {
            fs::write(FILE_OUT_FILENAME, "")?;
        }

        Ok(())
    }

    fn format_verf_helper(
        verf_type: VerfFile,
        verf_string: String,
        first_line_num: u32,
    ) -> TestResult {
        // Set up the verification items
        const FN_NAME: &str = "mt_logger::tests::format_verification";
        const VERF_MATRIX: [[&str; 3]; 6] = [
            ["TRACE", "030;105m", "  TRACE  "],
            ["DEBUG", "030;106m", "  DEBUG  "],
            ["INFO", "030;107m", "  INFO   "],
            ["WARNING", "030;103m", " WARNING "],
            ["ERROR", "030;101m", "  ERROR  "],
            ["FATAL", "031;040m", "  FATAL  "],
        ];
        const LEVEL_VERF_IDX: usize = 0;
        const COLOR_VERF_IDX: usize = 1;
        const PADDED_LEVEL_VERF_IDX: usize = 2;

        // Set up output-specific parameters
        let padded_level_hdr_capture_idx;
        let fn_name_hdr_capture_idx;
        let line_num_hdr_capture_idx;
        let header_regex;
        match verf_type {
            VerfFile::StdOut => {
                padded_level_hdr_capture_idx = STDOUT_PADDED_LEVEL_IDX;
                fn_name_hdr_capture_idx = STDOUT_FN_NAME_IDX;
                line_num_hdr_capture_idx = STDOUT_LINE_NUM_IDX;
                header_regex = Regex::new(STDOUT_HDR_REGEX_STR)?;
            }
            VerfFile::FileOut => {
                padded_level_hdr_capture_idx = FILE_OUT_PADDED_LEVEL_IDX;
                fn_name_hdr_capture_idx = FILE_OUT_FN_NAME_IDX;
                line_num_hdr_capture_idx = FILE_OUT_LINE_NUM_IDX;
                header_regex = Regex::new(FILE_OUT_HDR_REGEX_STR)?;
            }
        }
        let level_content_capture_idx = 1;

        // Create regex for message content
        let content_regex = Regex::new(r"^   This is an? (\w*) message.")?;

        // Read verf string into iterator
        let mut verf_lines: Vec<&str> = verf_string.split('\n').collect();
        let mut verf_line_iter = verf_lines.iter_mut();

        // Iterate over lines, verifying along the way
        let mut i = 0;
        while let Some(header_line) = verf_line_iter.next().filter(|v| !v.is_empty()) {
            // Match regex against header line, and capture groups
            let header_captures = header_regex.captures(header_line).unwrap_or_else(|| {
                panic!(
                    "{:?}: Header line {} '{}' did not match Regex:\n   {}",
                    verf_type,
                    i,
                    header_line,
                    header_regex.as_str()
                )
            });

            // Verify capture groups
            if verf_type == VerfFile::StdOut
                && &header_captures[STDOUT_COLOR_IDX] != VERF_MATRIX[i][COLOR_VERF_IDX]
            {
                panic!(
                    "{:?}: Wrong color '{}' on line '{}', should be '{}'",
                    verf_type,
                    &header_captures[STDOUT_COLOR_IDX],
                    header_line,
                    VERF_MATRIX[i][COLOR_VERF_IDX]
                );
            }
            if &header_captures[padded_level_hdr_capture_idx]
                != VERF_MATRIX[i][PADDED_LEVEL_VERF_IDX]
            {
                panic!(
                    "{:?}: Wrong padded level '{}' on line '{}', should be '{}'",
                    verf_type,
                    &header_captures[padded_level_hdr_capture_idx],
                    header_line,
                    VERF_MATRIX[i][PADDED_LEVEL_VERF_IDX]
                );
            }
            if &header_captures[fn_name_hdr_capture_idx] != FN_NAME {
                panic!(
                    "{:?}: Wrong function name '{}' on line '{}', should be '{}'",
                    verf_type, &header_captures[fn_name_hdr_capture_idx], header_line, FN_NAME
                );
            }
            if header_captures[line_num_hdr_capture_idx].parse::<u32>()?
                != first_line_num + i as u32
            {
                panic!(
                    "{:?}: Wrong line number '{}' on line '{}', should be '{}'",
                    verf_type,
                    &header_captures[line_num_hdr_capture_idx],
                    header_line,
                    first_line_num + i as u32
                );
            }

            // Verify content line
            let content_line = verf_line_iter
                .next()
                .unwrap_or_else(|| panic!("Missing content line after header '{}'", header_line));
            let content_captures = content_regex.captures(content_line).unwrap_or_else(|| {
                panic!(
                    "{:?}: Content line {} '{}' did not match content Regex:\n   {}",
                    verf_type,
                    i,
                    content_line,
                    content_regex.as_str()
                )
            });

            if &content_captures[level_content_capture_idx] != VERF_MATRIX[i][LEVEL_VERF_IDX] {
                panic!(
                    "{:?}: Wrong level '{}' in content line '{}', should be '{}'",
                    verf_type,
                    &content_captures[level_content_capture_idx],
                    content_line,
                    VERF_MATRIX[i][LEVEL_VERF_IDX]
                )
            }

            i += 1;
        }

        Ok(())
    }

    #[test]
    fn format_verification() -> TestResult {
        // Lock logger mutex and hold it until we're done processing messages
        let mutex = LOGGER_MUTEX.lock()?;

        // Clean verification files before test
        reset_verf_files()?;

        // Create or update logger instance such that all messages are logged to Both outputs
        if INSTANCE.get().is_none() {
            mt_new!(LOGFILE_PREFIX, Level::Trace, OutputStream::Both);
        } else {
            mt_level!(Level::Trace);
            mt_stream!(OutputStream::Both);
        }

        let first_line_num = line!() + 1;
        mt_log!(Level::Trace, "This is a TRACE message.");
        mt_log!(Level::Debug, "This is a DEBUG message.");
        mt_log!(Level::Info, "This is an INFO message.");
        mt_log!(Level::Warning, "This is a WARNING message.");
        mt_log!(Level::Error, "This is an ERROR message.");
        mt_log!(Level::Fatal, "This is a FATAL message.");

        // Flush the messages to their output
        println!("Flushing all messages to their output...");
        let start_time = time::Instant::now();
        mt_flush!()?;
        println!("Done flushing after {}ms", start_time.elapsed().as_millis());

        // Capture the files in memory before releasing the mutex
        let mut verf_file_stdout = fs::OpenOptions::new().read(true).open(STDOUT_FILENAME)?;
        let mut verf_string_stdout = String::new();
        verf_file_stdout.read_to_string(&mut verf_string_stdout)?;
        let mut verf_file_file_out = fs::OpenOptions::new().read(true).open(FILE_OUT_FILENAME)?;
        let mut verf_string_file_out = String::new();
        verf_file_file_out.read_to_string(&mut verf_string_file_out)?;

        // Unlock the mutex
        std::mem::drop(mutex);

        // Verify that the verification files contain well-formatted messages
        format_verf_helper(VerfFile::StdOut, verf_string_stdout, first_line_num)?;
        format_verf_helper(VerfFile::FileOut, verf_string_file_out, first_line_num)?;

        Ok(())
    }

    fn outputstream_verf_helper(verf_type: VerfFile, verf_string: String) -> TestResult {
        // Set up the verification items
        const VERF_MATRIX: [[[&str; 2]; 4]; 2] = [
            [
                ["TRACE", "BOTH"],
                ["FATAL", "BOTH"],
                ["TRACE", "STDOUT"],
                ["FATAL", "STDOUT"],
            ],
            [
                ["TRACE", "BOTH"],
                ["FATAL", "BOTH"],
                ["TRACE", "FILEOUT"],
                ["FATAL", "FILEOUT"],
            ],
        ];
        const STDOUT_TYPE_IDX: usize = 0;
        const FILE_OUT_TYPE_IDX: usize = 1;
        const LEVEL_VERF_IDX: usize = 0;
        const OUTPUT_TYPE_VERF_IDX: usize = 1;

        // Set up output-specific parameters
        let verf_type_idx;
        let padless_level_hdr_capture_idx;
        let header_regex;
        match verf_type {
            VerfFile::StdOut => {
                verf_type_idx = STDOUT_TYPE_IDX;
                padless_level_hdr_capture_idx = STDOUT_PADLESS_LEVEL_IDX;
                header_regex = Regex::new(STDOUT_HDR_REGEX_STR)?;
            }
            VerfFile::FileOut => {
                verf_type_idx = FILE_OUT_TYPE_IDX;
                padless_level_hdr_capture_idx = FILE_OUT_PADLESS_LEVEL_IDX;
                header_regex = Regex::new(FILE_OUT_HDR_REGEX_STR)?;
            }
        }
        let output_type_capture_idx = 1;

        // Create regex for message content
        let content_regex = Regex::new(r"^\s*This message appears in (\w*).")?;

        // Read verf string into iterator
        let mut verf_lines: Vec<&str> = verf_string.split('\n').collect();
        let mut verf_line_iter = verf_lines.iter_mut();

        // Verify that the verification files contain the correct filter level and content lines
        let mut i = 0;
        while let Some(header_line) = verf_line_iter.next().filter(|v| !v.is_empty()) {
            // Verify header contains the correct log level
            let header_captures = header_regex.captures(header_line).unwrap_or_else(|| {
                panic!(
                    "{:?}: Header line {} '{}' did not match Regex:\n   {}",
                    verf_type,
                    i,
                    header_line,
                    header_regex.as_str()
                )
            });
            if &header_captures[padless_level_hdr_capture_idx]
                != VERF_MATRIX[verf_type_idx][i][LEVEL_VERF_IDX]
            {
                panic!(
                    "{:?}: Wrong level '{}' on line '{}', should be '{}'",
                    verf_type,
                    &header_captures[padless_level_hdr_capture_idx],
                    header_line,
                    VERF_MATRIX[verf_type_idx][i][LEVEL_VERF_IDX]
                );
            }

            // Verify content contains the correct output type
            let content_line = verf_line_iter
                .next()
                .unwrap_or_else(|| panic!("Missing content line after header '{}'", header_line));
            let content_captures = content_regex.captures(content_line).unwrap_or_else(|| {
                panic!(
                    "{:?}: Content line {} '{}' did not match content Regex:\n   {}",
                    verf_type,
                    i,
                    content_line,
                    content_regex.as_str()
                )
            });

            if &content_captures[output_type_capture_idx]
                != VERF_MATRIX[verf_type_idx][i][OUTPUT_TYPE_VERF_IDX]
            {
                panic!(
                    "{:?}: Wrong output type '{}' on line '{}', should be '{}'",
                    verf_type,
                    &content_captures[output_type_capture_idx],
                    content_line,
                    VERF_MATRIX[verf_type_idx][i][OUTPUT_TYPE_VERF_IDX]
                )
            }

            i += 1;
        }

        Ok(())
    }

    #[test]
    fn outputstream_verification() -> TestResult {
        // Lock logger mutex and hold it until we're done processing messages
        let mutex = LOGGER_MUTEX.lock()?;

        // Clean verification files before test
        reset_verf_files()?;

        // Create or update logger instance such that all messages are logged to Both outputs
        if INSTANCE.get().is_none() {
            mt_new!(LOGFILE_PREFIX, Level::Trace, OutputStream::Both);
        } else {
            mt_level!(Level::Trace);
            mt_stream!(OutputStream::Both);
        }

        mt_log!(Level::Trace, "This message appears in BOTH.");
        mt_log!(Level::Fatal, "This message appears in BOTH.");

        // Log messages to STDOUT only
        mt_stream!(OutputStream::StdOut);
        mt_log!(Level::Trace, "This message appears in STDOUT.");
        mt_log!(Level::Fatal, "This message appears in STDOUT.");

        // Log messages to FILE only
        mt_stream!(OutputStream::File);
        mt_log!(Level::Trace, "This message appears in FILEOUT.");
        mt_log!(Level::Fatal, "This message appears in FILEOUT.");

        // Log messages to NEITHER output
        mt_stream!(OutputStream::Neither);
        mt_log!(Level::Trace, "This message appears in NEITHER.");
        mt_log!(Level::Fatal, "This message appears in NEITHER.");

        // Flush the messages to their output
        println!("Flushing all messages to their output...");
        let start_time = time::Instant::now();
        mt_flush!()?;
        println!("Done flushing after {}ms", start_time.elapsed().as_millis());

        // Capture the files in memory before releasing the mutex
        let mut verf_file_stdout = fs::OpenOptions::new().read(true).open(STDOUT_FILENAME)?;
        let mut verf_string_stdout = String::new();
        verf_file_stdout.read_to_string(&mut verf_string_stdout)?;
        let mut verf_file_file_out = fs::OpenOptions::new().read(true).open(FILE_OUT_FILENAME)?;
        let mut verf_string_file_out = String::new();
        verf_file_file_out.read_to_string(&mut verf_string_file_out)?;

        // Unlock the mutex
        std::mem::drop(mutex);

        // Verify that the verification files contain only the correct messages
        outputstream_verf_helper(VerfFile::StdOut, verf_string_stdout)?;
        outputstream_verf_helper(VerfFile::FileOut, verf_string_file_out)?;

        Ok(())
    }

    #[test]
    fn flush_test() -> TestResult {
        // Lock logger mutex and hold it for the remainder of this test
        let _mutex = LOGGER_MUTEX.lock()?;

        // Clean verification files before test
        reset_verf_files()?;

        // Set up the logger instance
        if INSTANCE.get().is_none() {
            mt_new!(LOGFILE_PREFIX, Level::Info, OutputStream::StdOut);
        } else {
            mt_level!(Level::Info);
            mt_stream!(OutputStream::StdOut);
        }

        // Capture the initial count
        let initial_msg_count = mt_count!();
        eprintln!("Initial message count: {}", initial_msg_count);

        // Send some messages
        eprintln!("Sending messages...");
        let sent_msg_count = 5;
        for i in 0..sent_msg_count {
            mt_log!(Level::Info, "Message #{}", i);
        }

        // Send a flush command
        mt_flush!()?;

        // Verify that all sent messages were processed after flushing
        eprintln!("Messages processed: {}", mt_count!());
        assert_eq!(initial_msg_count + sent_msg_count, mt_count!());

        Ok(())
    }
}
