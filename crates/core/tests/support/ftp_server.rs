use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread;

pub fn origin(partial: bool) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let start = std::time::Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(start.elapsed() < std::time::Duration::from_secs(10));
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut control = BufReader::new(stream);
        fn reply(control: &mut BufReader<std::net::TcpStream>, line: &str) {
            control.get_mut().write_all(line.as_bytes()).unwrap();
        }
        fn command(control: &mut BufReader<std::net::TcpStream>, expected: &str) {
            let mut text = String::new();
            control.read_line(&mut text).unwrap();
            assert_eq!(text, format!("{expected}\r\n"));
        }
        reply(&mut control, "220 Ready\r\n");
        command(&mut control, "USER anonymous");
        reply(&mut control, "230 Accepted\r\n");
        command(&mut control, "TYPE I");
        reply(&mut control, "200 Binary\r\n");
        command(&mut control, "SIZE pub/a.bin");
        reply(&mut control, "213 8\r\n");
        command(&mut control, "EPSV");
        let data_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        reply(
            &mut control,
            &format!(
                "229 Passive (|||{}|)\r\n",
                data_listener.local_addr().unwrap().port()
            ),
        );
        command(&mut control, "RETR pub/a.bin");
        reply(&mut control, "150 Data\r\n");
        let (mut data, _) = data_listener.accept().unwrap();
        data.write_all(if partial { b"first" } else { b"firstend" })
            .unwrap();
        drop(data);
        if !partial {
            reply(&mut control, "226 Complete\r\n");
            command(&mut control, "SIZE pub/a.bin");
            reply(&mut control, "213 8\r\n");
            assert_eq!(control.read(&mut [0]).unwrap(), 0);
        }
    });
    (endpoint, worker)
}
