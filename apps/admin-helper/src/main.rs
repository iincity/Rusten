fn main() {
    let mut handler = admin_helper::request_handler::RequestHandler::default();
    loop {
        match admin_helper::ipc_server::serve_once(&mut handler) {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => {
                eprintln!("admin helper stopped: {error}");
                std::process::exit(1);
            }
        }
    }
}
