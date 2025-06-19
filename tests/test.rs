use mt_logger::*; // or import specific things like your macros, logger, etc.

#[test]
fn test_logging() {
    mt_new!(None, Level::Trace, OutputStream::StdOut, true); // assuming Level and OutputStream are public

    mt_log!(Level::Debug, "Hello from test!");
}
