//! Two rinch-rtc peers in one window, talking to each other over real
//! WebRTC data channels on this machine. Signalling, which an app normally
//! carries over its own WebSocket, is a direct hand-off here.
//!
//! - "chat": in-band and reliable. Peer A opens it and B hears it in
//!   `on_data_channel`. B echoes every message back.
//! - "tick": pre-negotiated (id 1) and unreliable (no retransmits). A sends
//!   one every 100 ms.
//!
//! Both peers are created while the component renders, so they close when
//! the window closes.

use rinch::prelude::*;
use rinch_rtc::{Channel, ChannelOptions, Peer, PeerState, Reliability, RtcConfig, SendError};

#[component]
fn app() -> NodeHandle {
    let state_a = Signal::new(String::from("new"));
    let state_b = Signal::new(String::from("new"));
    let log = Signal::new(Vec::<String>::new());
    let ticks = Signal::new(0u64);
    let draft = Signal::new(String::new());

    let a = Peer::new(RtcConfig::default()).expect("peer A");
    let b = Peer::new(RtcConfig::default()).expect("peer B");
    a.on_connection_state(move |s: PeerState| state_a.set(format!("{s:?}")));
    b.on_connection_state(move |s: PeerState| state_b.set(format!("{s:?}")));

    // Trickle each side's candidates straight to the other.
    let wb = b.downgrade();
    a.on_ice_candidate(move |c| {
        if let Some(b) = wb.upgrade() {
            b.add_ice_candidate(c)
        }
    });
    let wa = a.downgrade();
    b.on_ice_candidate(move |c| {
        if let Some(a) = wa.upgrade() {
            a.add_ice_candidate(c)
        }
    });

    let chat = a
        .create_data_channel("chat", ChannelOptions::new(Reliability::Reliable))
        .expect("chat");
    chat.on_message(move |bytes| {
        log.update(|l| l.push(format!("A got back: {}", String::from_utf8_lossy(&bytes))))
    });
    let tick_opts = ChannelOptions::negotiated(1, Reliability::Unreliable { max_retransmits: 0 });
    let tick_a = a.create_data_channel("tick", tick_opts).expect("tick A");
    let tick_b = b.create_data_channel("tick", tick_opts).expect("tick B");
    tick_b.on_message(move |_| ticks.update(|t| *t += 1));

    // B echoes whatever arrives on the channel A opened.
    let kept: Signal<Option<Channel>> = Signal::new(None);
    b.on_data_channel(move |ch| {
        let echo = ch.clone();
        ch.on_message(move |bytes| {
            log.update(|l| l.push(format!("B got: {}", String::from_utf8_lossy(&bytes))));
            let _ = echo.send(bytes);
        });
        kept.set(Some(ch));
    });

    // Offer and answer.
    let (pa, pb) = (a.clone(), b.clone());
    spawn_local(async move {
        let Ok(offer) = pa.create_offer().await else {
            return;
        };
        if pb.set_remote_description(offer).await.is_err() {
            return;
        }
        let Ok(answer) = pb.create_answer().await else {
            return;
        };
        let _ = pa.set_remote_description(answer).await;
    });

    // A peer closes when its last handle drops. The callbacks above hold only
    // weak handles, and the task above lets go of its clones once it is done,
    // so keep both peers for as long as this component lives.
    let (keep_a, keep_b) = (a.clone(), b.clone());
    rinch_core::reactive::on_cleanup(move || drop((keep_a, keep_b)));

    // A tick every 100 ms; when the queue is full, a tick is simply skipped.
    let t = tick_a.clone();
    set_interval(100, move || match t.send(vec![0u8; 32]) {
        Ok(()) | Err(SendError::NotOpen) | Err(SendError::QueueFull { .. }) => {}
        Err(e) => eprintln!("tick: {e}"),
    });

    let send = {
        let chat = chat.clone();
        move || {
            let text = draft.get();
            if text.is_empty() {
                return;
            }
            match chat.send(text.clone().into_bytes()) {
                Ok(()) => draft.set(String::new()),
                Err(e) => log.update(|l| l.push(format!("not sent: {e}"))),
            }
        }
    };
    let send2 = send.clone();

    rsx! {
        Stack { gap: "md", p: "lg",
            Title { order: 2, "rinch-rtc loopback" }
            Text { {move || format!("peer A: {}   peer B: {}", state_a.get(), state_b.get())} }
            Text { color: "dimmed", {move || format!("ticks B received: {}", ticks.get())} }
            Group { gap: "sm",
                TextInput {
                    placeholder: "A message for B",
                    value_fn: move || draft.get(),
                    oninput: move |v: String| draft.set(v),
                    onsubmit: send,
                    style: "flex: 1;",
                }
                Button { variant: "filled", onclick: send2, "Send" }
            }
            Stack { gap: "xs",
                for line in log.get() {
                    Text { size: "sm", {line.clone()} }
                }
            }
        }
    }
}

fn main() {
    App::new(app)
        .title("rinch-rtc loopback")
        .size(520, 600)
        .run();
}
