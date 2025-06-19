use mt_logger::*; // or import specific things like your macros, logger, etc.

#[test]
fn test_logging() {
    mt_new!(None, Level::Trace, OutputStream::StdOut, false); // assuming Level and OutputStream are public

    mt_log!(Level::Debug, "Hello from test!");
    mt_log!(Level::Debug, "Hello from test!");

    mt_flush!(); // Ensure logs are flushed to the output
}
