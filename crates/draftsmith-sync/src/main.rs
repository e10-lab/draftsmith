//! `dsmith-sync` — DraftSmith 공동작업 중계 서버 (구현 노트 §90).
//!
//! `dsmith-sync --port 7878 [--dir 저장폴더]` 로 띄우면, 앱의 「공동작업」 이 `ws://주소:7878` 로 붙는다.
//! 방(도면 이름)마다 기준 편집기를 들고 들어온 작업을 고유 번호로 맞춰 나눈다 (`draftsmith_core::sync`).
//! `--dir` 을 주면 방 도면을 `방이름.dsmith` 로 10 초마다 저장하고, 다시 띄우면 이어서 연다.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use draftsmith_core::sync::{Msg, Room};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(name = "dsmith-sync", about = "DraftSmith 공동작업 중계 서버")]
struct Cli {
    /// 받을 주소 (같은 망의 다른 컴퓨터가 붙으려면 0.0.0.0)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 7878)]
    port: u16,
    /// 방 도면을 저장할 폴더
    #[arg(long)]
    dir: Option<PathBuf>,
}

struct RoomState {
    room: Room,
    peers: HashMap<u64, mpsc::UnboundedSender<String>>,
    dirty: bool,
}

#[derive(Default)]
struct Hub {
    rooms: HashMap<String, RoomState>,
    next_peer: u64,
    dir: Option<PathBuf>,
}

type Shared = Arc<Mutex<Hub>>;

/// 방 이름 → 파일 이름에 쓸 수 있는 글자만
fn file_of(dir: &std::path::Path, room: &str) -> PathBuf {
    let safe: String = room
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}.dsmith"))
}

fn text(m: &Msg) -> String {
    serde_json::to_string(m).unwrap_or_default()
}

/// 방 사람들에게 (보낸 사람 빼고)
fn broadcast(st: &RoomState, except: Option<u64>, m: &Msg) {
    let t = text(m);
    for (id, tx) in &st.peers {
        if Some(*id) != except {
            let _ = tx.send(t.clone());
        }
    }
}

async fn serve(stream: TcpStream, hub: Shared) -> anyhow::Result<()> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    // 보낼 말은 한 곳에서 차례로
    let writer = tokio::spawn(async move {
        while let Some(t) = rx.recv().await {
            if sink.send(Message::Text(t.into())).await.is_err() {
                break;
            }
        }
    });
    let mut joined: Option<(String, u64)> = None;
    while let Some(m) = source.next().await {
        let m = m?;
        let Message::Text(t) = m else {
            if m.is_close() {
                break;
            }
            continue;
        };
        let msg: Msg = match serde_json::from_str(&t) {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(text(&Msg::Error {
                    msg: format!("알 수 없는 말: {e}"),
                }));
                continue;
            }
        };
        let mut h = hub.lock().await;
        match (msg, &joined) {
            (Msg::Create { room, drawing }, None) => {
                let id = enter(&mut h, &room, Some(*drawing), tx.clone())?;
                joined = Some((room, id));
            }
            (Msg::Join { room }, None) => match enter(&mut h, &room, None, tx.clone()) {
                Ok(id) => joined = Some((room, id)),
                Err(e) => {
                    let _ = tx.send(text(&Msg::Error { msg: e.to_string() }));
                }
            },
            (Msg::Op { n, op }, Some((room, id))) => {
                let Some(st) = h.rooms.get_mut(room) else {
                    continue;
                };
                let (me, others) = st.room.handle(n, &op);
                let _ = tx.send(text(&me));
                if let Some(o) = others {
                    st.dirty = true;
                    broadcast(st, Some(*id), &o);
                }
            }
            _ => {
                let _ = tx.send(text(&Msg::Error {
                    msg: "먼저 방을 만들거나 들어가세요".into(),
                }));
            }
        }
    }
    // 나감
    if let Some((room, id)) = joined {
        let mut h = hub.lock().await;
        if let Some(st) = h.rooms.get_mut(&room) {
            st.peers.remove(&id);
            broadcast(st, None, &Msg::People { n: st.peers.len() });
        }
    }
    writer.abort();
    Ok(())
}

/// 방에 들어간다 (없으면 `drawing` 으로, 그것도 없으면 저장 폴더에서). 사람 번호
fn enter(
    h: &mut Hub,
    room: &str,
    drawing: Option<draftsmith_core::Drawing>,
    tx: mpsc::UnboundedSender<String>,
) -> anyhow::Result<u64> {
    if !h.rooms.contains_key(room) {
        let d = match (drawing, &h.dir) {
            (Some(d), _) => d,
            (None, Some(dir)) if file_of(dir, room).exists() => {
                draftsmith_core::docfile::load(&file_of(dir, room)).map_err(anyhow::Error::msg)?
            }
            _ => anyhow::bail!("「{room}」 방이 없습니다"),
        };
        h.rooms.insert(
            room.to_string(),
            RoomState {
                room: Room::new(d),
                peers: HashMap::new(),
                dirty: true,
            },
        );
    }
    h.next_peer += 1;
    let id = h.next_peer;
    let st = h.rooms.get_mut(room).unwrap();
    st.peers.insert(id, tx.clone());
    let _ = tx.send(text(&Msg::Welcome {
        drawing: Box::new(st.room.ed.d.clone()),
        people: st.peers.len(),
    }));
    broadcast(st, Some(id), &Msg::People { n: st.peers.len() });
    Ok(id)
}

/// 바뀐 방 도면을 저장 폴더에
async fn saver(hub: Shared) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let mut h = hub.lock().await;
        let Some(dir) = h.dir.clone() else { return };
        for (name, st) in h.rooms.iter_mut().filter(|(_, s)| s.dirty) {
            match draftsmith_core::docfile::save(&st.room.ed.d, &file_of(&dir, name)) {
                Ok(()) => st.dirty = false,
                Err(e) => eprintln!("「{name}」 저장 실패: {e}"),
            }
        }
    }
}

/// 서버를 띄운다 (시험에서도 쓴다)
async fn run(listener: TcpListener, dir: Option<PathBuf>) {
    let hub: Shared = Arc::new(Mutex::new(Hub {
        dir,
        ..Default::default()
    }));
    tokio::spawn(saver(hub.clone()));
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let hub = hub.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(stream, hub).await {
                eprintln!("연결 끊김: {e}");
            }
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(d) = &cli.dir {
        std::fs::create_dir_all(d)?;
    }
    let listener = TcpListener::bind((cli.host.as_str(), cli.port)).await?;
    println!("DraftSmith 공동작업 서버: ws://{}:{}", cli.host, cli.port);
    run(listener, cli.dir).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use draftsmith_core::Shape;
    use draftsmith_core::edit::Editor;
    use draftsmith_core::geom::{Affine, Vec2};
    use draftsmith_core::sync::Client;
    use tungstenite::connect;

    fn line(y: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(0.0, y),
            b: Vec2::new(1000.0, y),
        }
    }

    type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

    fn send(ws: &mut Ws, m: &Msg) {
        ws.send(tungstenite::Message::Text(text(m).into())).unwrap();
    }
    fn recv(ws: &mut Ws) -> Msg {
        loop {
            if let tungstenite::Message::Text(t) = ws.read().unwrap() {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }
    /// 받을 말이 `pred` 일 때까지 받으며 편집기에 적용
    fn pump(ws: &mut Ws, c: &mut Client, ed: &mut Editor, pred: impl Fn(&Msg) -> bool) {
        loop {
            let m = recv(ws);
            c.receive(ed, &m);
            if pred(&m) {
                return;
            }
        }
    }

    /// 실제 소켓으로: 갑이 방을 만들고 을이 들어와, 둘이 고친 것이 모두 같아진다
    #[test]
    fn two_clients_over_websocket() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let addr = listener.local_addr().unwrap();
        rt.spawn(run(listener, None));
        let url = format!("ws://{addr}");

        let mut base = Editor::new(draftsmith_core::archgen::new_drawing());
        base.add_shapes(vec![line(0.0), line(100.0)]);
        // 갑: 방 만들기
        let (mut a, _) = connect(&url).unwrap();
        send(
            &mut a,
            &Msg::Create {
                room: "평면".into(),
                drawing: Box::new(base.d.clone()),
            },
        );
        let Msg::Welcome { drawing, people } = recv(&mut a) else {
            panic!()
        };
        assert_eq!(people, 1);
        let mut ea = Editor::new(*drawing);
        ea.d.salt_ids(1);
        // 을: 들어오기
        let (mut b, _) = connect(&url).unwrap();
        send(
            &mut b,
            &Msg::Join {
                room: "평면".into(),
            },
        );
        let Msg::Welcome { drawing, people } = recv(&mut b) else {
            panic!()
        };
        assert_eq!(people, 2);
        let mut eb = Editor::new(*drawing);
        eb.d.salt_ids(2);
        let (mut ca, mut cb) = (Client::default(), Client::default());
        // 갑: 선 하나 옮기고, 을: 선 하나 더하기
        ea.transform(&[0], &Affine::translate(0.0, 5.0));
        let m = ca.local(ea.history().last().unwrap()).unwrap();
        send(&mut a, &m);
        eb.add_shapes(vec![line(300.0)]);
        let m = cb.local(eb.history().last().unwrap()).unwrap();
        send(&mut b, &m);
        // 각자 자기 Ack 와 상대 작업을 받을 때까지
        let mut got_a = (false, false);
        while got_a != (true, true) {
            let m = recv(&mut a);
            ca.receive(&mut ea, &m);
            match m {
                Msg::Ack { .. } => got_a.0 = true,
                Msg::Applied { .. } => got_a.1 = true,
                _ => {}
            }
        }
        pump(&mut b, &mut cb, &mut eb, |m| {
            matches!(m, Msg::Applied { .. })
        });
        let key = |d: &draftsmith_core::Drawing| {
            let mut v: Vec<String> = d
                .entities
                .iter()
                .map(|e| serde_json::to_string(e).unwrap())
                .collect();
            v.sort();
            v
        };
        assert_eq!(key(&ea.d), key(&eb.d));
        assert_eq!(ea.d.entities.len(), 3);
        // 없는 방
        let (mut c, _) = connect(&url).unwrap();
        send(
            &mut c,
            &Msg::Join {
                room: "없는 방".into(),
            },
        );
        assert!(matches!(recv(&mut c), Msg::Error { .. }));
    }
}
