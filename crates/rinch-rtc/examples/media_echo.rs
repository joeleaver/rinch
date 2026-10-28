//! A native rinch-rtc media peer behind a tiny HTTP signalling endpoint:
//! what rinch-rtc's browser ↔ native test connects to (design record
//! `docs/design/rtc-media.md`, section 6). It is the codec interop test:
//! openh264 decoding what a browser's H.264 encoder sends, and the browser
//! decoding what openh264 sends back.
//!
//! `POST /offer` with an SDP offer as the body answers with the SDP answer
//! (CORS open, non-trickle: the answer carries the candidates). Each offer
//! gets its own peer, which decodes the first video track it receives and
//! sends every frame back, re-encoded, on a video track of its own. Audio
//! is received and played into the mixer only (no device needed: the mixer
//! is in capture mode).
//!
//! ```text
//! cargo run -p rinch-rtc --features media --example media_echo -- [--port 0] [--url-file PATH] [--h264-only]
//! ```
//!
//! Built with `media` it offers and accepts H.264 alone; with
//! `--features media,media-vp8`, VP8 first and H.264, and `--h264-only`
//! drops VP8 again.
//!
//! It prints `media_echo http://127.0.0.1:<port>` (and writes the URL to
//! `--url-file`).

fn main() {
    #[cfg(all(not(target_arch = "wasm32"), feature = "media-core"))]
    app::main();
    #[cfg(not(all(not(target_arch = "wasm32"), feature = "media-core")))]
    eprintln!("media_echo needs a desktop build with --features media");
}

#[cfg(all(not(target_arch = "wasm32"), feature = "media-core"))]
mod app {
    use std::future::Future;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::pin::Pin;
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use std::task::{Context, Waker};
    use std::time::Duration;

    use rinch_rtc::media::{LocalTrack, MediaKind, VideoSourceOptions};
    use rinch_rtc::{Description, Gathering, Peer, RtcConfig};

    type Job = Box<dyn FnOnce() + Send>;
    static QUEUE: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();

    /// An offer from the HTTP thread and where its answer goes.
    type Offer = (String, mpsc::Sender<Result<String, String>>);
    type Task = Pin<Box<dyn Future<Output = ()>>>;

    pub fn main() {
        let mut port = 0u16;
        let mut url_file = None;
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--port" => port = args.next().and_then(|p| p.parse().ok()).expect("--port N"),
                "--url-file" => url_file = args.next(),
                // H.264 only, no VP8: a browser offers VP8 first, so in a
                // `media-vp8` build this is how its H.264 is exercised (a
                // `media` build is H.264-only already).
                "--h264-only" => rinch_rtc::testing::h264_only(true),
                other => panic!("unknown argument {other}"),
            }
        }
        // This thread is the "main thread" rinch-rtc reports to.
        QUEUE.set(Mutex::new(Vec::new())).ok();
        rinch_core::register_main_thread();
        rinch_core::set_cross_thread_dispatcher(|f| QUEUE.get().unwrap().lock().unwrap().push(f));
        // Remote audio goes to the mixer, not a device.
        let _mixed = rinch_rtc::testing::capture_audio_output();

        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind the HTTP port");
        let url = format!("http://{}", listener.local_addr().unwrap());
        println!("media_echo {url}");
        if let Some(path) = url_file {
            std::fs::write(path, &url).expect("write --url-file");
        }
        let (offers_tx, offers) = mpsc::channel::<Offer>();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Err(e) = serve(stream, &offers_tx) {
                    eprintln!("media_echo: {e}");
                }
            }
        });

        let mut peers: Vec<Peer> = Vec::new();
        let mut tasks: Vec<Task> = Vec::new();
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            let jobs: Vec<Job> = QUEUE.get().unwrap().lock().unwrap().drain(..).collect();
            for job in jobs {
                job();
            }
            while let Ok((sdp, reply)) = offers.try_recv() {
                match echo_peer() {
                    Ok(peer) => {
                        tasks.push(Box::pin(answer(peer.clone(), sdp, reply)));
                        peers.push(peer);
                        if peers.len() > 8 {
                            peers.remove(0).close();
                        }
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            tasks.retain_mut(|t| t.as_mut().poll(&mut cx).is_pending());
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A peer that sends back every video frame it receives.
    fn echo_peer() -> Result<Peer, String> {
        let peer = Peer::new(RtcConfig {
            gathering: Gathering::Complete,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        let (track, writer) = LocalTrack::video_source(VideoSourceOptions {
            width: 320,
            height: 240,
            fps: 30,
        })
        .map_err(|e| e.to_string())?;
        peer.add_track(&track).map_err(|e| e.to_string())?;
        // The track lives as long as the peer's callbacks do.
        let keep = track.clone();
        peer.on_track(move |remote| {
            let _ = &keep;
            if remote.kind() == MediaKind::Video {
                let w = writer.clone();
                remote.on_frame(move |rgba, width, height| w.push_rgba(rgba, width, height));
            }
        });
        peer.on_error(|e| eprintln!("media_echo: peer: {e}"));
        Ok(peer)
    }

    async fn answer(peer: Peer, offer: String, reply: mpsc::Sender<Result<String, String>>) {
        let result = async {
            peer.set_remote_description(Description::offer(offer))
                .await
                .map_err(|e| e.to_string())?;
            let answer = peer.create_answer().await.map_err(|e| e.to_string())?;
            Ok::<_, String>(answer.sdp)
        }
        .await;
        let _ = reply.send(result);
    }

    fn serve(stream: TcpStream, offers: &mpsc::Sender<Offer>) -> std::io::Result<()> {
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
                let (tx, rx) = mpsc::channel();
                let _ = offers.send((String::from_utf8_lossy(&body).into_owned(), tx));
                match rx.recv_timeout(Duration::from_secs(20)) {
                    Ok(Ok(answer)) => ("200 OK", answer),
                    Ok(Err(e)) => ("500 Internal Server Error", e),
                    Err(_) => ("504 Gateway Timeout", "no answer".to_string()),
                }
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
