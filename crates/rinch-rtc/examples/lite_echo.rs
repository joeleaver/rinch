//! A str0m ICE-lite echo peer behind a tiny HTTP signalling endpoint: the
//! server rinch-rtc's browser tests connect to (design record section 4).
//!
//! `POST /offer` with an SDP offer as the body answers with the SDP answer
//! (CORS open, so a test page on another port may call it). Each offer gets
//! its own lite peer: ICE-lite, one UDP port, one host candidate, echoing
//! every message. Shaped like Playweft's hosted game server.
//!
//! ```text
//! cargo run -p rinch-rtc --example lite_echo -- [--port 0] [--url-file PATH]
//! ```
//!
//! It prints `lite_echo http://127.0.0.1:<port>` (and writes the URL to
//! `--url-file`). The UDP side binds `0.0.0.0` and advertises this machine's
//! primary IPv4 address: Chrome offers no loopback candidates of its own, so a
//! peer it is to reach is not advertised on 127.0.0.1.

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    app::main();
}

#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/common/lite.rs"]
mod lite;

#[cfg(not(target_arch = "wasm32"))]
mod app {

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};

    use super::lite::LitePeer;

    /// The address the routing table would send from: no packet is sent.
    fn primary_ipv4() -> IpAddr {
        UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| {
                s.connect("192.0.2.1:9")?;
                s.local_addr()
            })
            .map(|a| a.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    pub fn main() {
        let mut port = 0u16;
        let mut url_file = None;
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--port" => port = args.next().and_then(|p| p.parse().ok()).expect("--port N"),
                "--url-file" => url_file = args.next(),
                other => panic!("unknown argument {other}"),
            }
        }
        let ip = primary_ipv4();
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the HTTP port");
        let url = format!("http://{}", listener.local_addr().unwrap());
        println!("lite_echo {url} (UDP host candidates on {ip})");
        if let Some(path) = url_file {
            std::fs::write(path, &url).expect("write --url-file");
        }

        let mut peers: Vec<LitePeer> = Vec::new();
        for stream in listener.incoming().flatten() {
            if let Err(e) = serve(stream, ip, &mut peers) {
                eprintln!("lite_echo: {e}");
            }
            // Keep the newest few: a test run makes a handful of connections.
            if peers.len() > 16 {
                peers.remove(0);
            }
        }
    }

    fn serve(stream: TcpStream, ip: IpAddr, peers: &mut Vec<LitePeer>) -> std::io::Result<()> {
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body)?;
        let method = request_line.split_whitespace().next().unwrap_or("");
        let (status, reply) = match method {
            "OPTIONS" => ("204 No Content", String::new()),
            "POST" => {
                let socket = UdpSocket::bind("0.0.0.0:0")?;
                let advertise = SocketAddr::new(ip, socket.local_addr()?.port());
                let peer = LitePeer::spawn_on(socket, advertise);
                let answer = peer.answer(&String::from_utf8_lossy(&body));
                peers.push(peer);
                ("200 OK", answer)
            }
            _ => ("405 Method Not Allowed", String::new()),
        };
        let mut out = stream;
        write!(
            out,
            "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Allow-Private-Network: true\r\n\
         Content-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
            reply.len()
        )?;
        out.flush()
    }
}
