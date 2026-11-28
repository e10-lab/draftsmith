//! 실시간 공동작업 — 앱 쪽 연결 (구현 노트 §90).
//!
//! 서버(`dsmith-sync`)와 WebSocket 하나. 소켓은 스레드 하나가 맡아, 짧게(50 ms) 기다리며 읽고 그 사이 쌓인 보낼 말을 보낸다.
//! 받은 말을 편집기에 적용하는 일은 부르는 쪽(`on_msg`)이 한다.

use std::net::TcpStream;
use std::sync::mpsc;
use std::time::Duration;

use draftsmith_core::Drawing;
use draftsmith_core::sync::{Client, Msg};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

/// 공동작업 중 상태
pub struct Collab {
    /// 연결 번호 — 끊긴 옛 연결의 스레드가 새 연결을 지우지 않게
    pub id: u64,
    pub client: Client,
    tx: mpsc::Sender<Msg>,
    /// 편집기 작업 기록(`journal`) 중 이미 보낸 수
    pub sent: usize,
    pub room: String,
    pub people: usize,
}

impl Collab {
    pub fn new(id: u64, tx: mpsc::Sender<Msg>, room: String, people: usize) -> Collab {
        Collab {
            id,
            client: Client::default(),
            tx,
            sent: 0,
            room,
            people,
        }
    }

    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }
}

fn set_timeout(ws: &Ws, t: Option<Duration>) {
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(t);
    }
}

fn send(ws: &mut Ws, m: &Msg) -> Result<(), String> {
    let t = serde_json::to_string(m).map_err(|e| e.to_string())?;
    ws.send(Message::Text(t.into())).map_err(|e| e.to_string())
}

/// 서버에 붙어 `first`(방 만들기·들어가기)를 보내고 방 도면을 받는다: (도면, 사람 수, 소켓)
pub fn open(url: &str, first: &Msg) -> Result<(Drawing, usize, Ws), String> {
    let (mut ws, _) =
        tungstenite::connect(url).map_err(|e| format!("서버에 붙지 못했습니다 ({url}): {e}"))?;
    set_timeout(&ws, Some(Duration::from_secs(15)));
    send(&mut ws, first)?;
    loop {
        match ws.read().map_err(|e| format!("서버 답이 없습니다: {e}"))? {
            Message::Text(t) => match serde_json::from_str::<Msg>(&t) {
                Ok(Msg::Welcome { drawing, people }) => {
                    set_timeout(&ws, Some(Duration::from_millis(50)));
                    return Ok((*drawing, people, ws));
                }
                Ok(Msg::Error { msg }) => return Err(msg),
                _ => {}
            },
            Message::Close(_) => return Err("서버가 연결을 닫았습니다".into()),
            _ => {}
        }
    }
}

/// 소켓 돌리기: 받은 말마다 `on_msg` (거짓을 돌려주면 끝), 보낼 말은 `rx` 에서. 끊기면 돌아온다
pub fn run(mut ws: Ws, rx: mpsc::Receiver<Msg>, mut on_msg: impl FnMut(Msg) -> bool) {
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Ok(m) = serde_json::from_str::<Msg>(&t)
                    && !on_msg(m)
                {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        loop {
            match rx.try_recv() {
                Ok(m) => {
                    if send(&mut ws, &m).is_err() {
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = ws.close(None);
                    return;
                }
            }
        }
    }
}
