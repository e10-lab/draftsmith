//! 건축 객체 모델 (M3): 층·벽·개구부(문·창)·실·그리드.
//!
//! 도면에는 선이 아니라 이 객체들을 저장하고, 평면도 선·면적표·창호일람표는 [`crate::archgen`] 이 만든다
//! (구현 노트 §19). 단위는 도면 단위(mm).

use serde::{Deserialize, Serialize};

use crate::geom::Vec2;

/// 층
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub id: u64,
    /// 「1층」, 「지하1층」 …
    pub name: String,
    /// 바닥 높이 (mm, 기준층 0)
    pub elevation: f64,
    /// 층고 (mm)
    pub height: f64,
}

/// 벽 종류 (선·해치·두께 기본값이 다르다)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WallKind {
    /// 외벽 (콘크리트 + 단열)
    #[default]
    Exterior,
    /// 내벽 (경량·조적)
    Interior,
    /// 내력벽·코어 (콘크리트)
    Structural,
}

impl WallKind {
    pub fn label(self) -> &'static str {
        match self {
            WallKind::Exterior => "외벽",
            WallKind::Interior => "내벽",
            WallKind::Structural => "내력벽",
        }
    }
}

/// 벽 (중심선 a→b, 두께)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    pub id: u64,
    pub level: u64,
    pub a: Vec2,
    pub b: Vec2,
    pub thickness: f64,
    /// 벽 높이 (None 이면 층고)
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub kind: WallKind,
}

impl Wall {
    pub fn length(&self) -> f64 {
        self.a.dist(self.b)
    }

    /// 단위 방향 (a→b)
    pub fn dir(&self) -> Vec2 {
        let l = self.length().max(1e-12);
        Vec2::new((self.b.x - self.a.x) / l, (self.b.y - self.a.y) / l)
    }
}

/// 개구부 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpeningKind {
    /// 여닫이문
    Door,
    /// 창
    Window,
}

/// 벽에 낸 문·창
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opening {
    pub id: u64,
    pub wall: u64,
    pub kind: OpeningKind,
    /// 벽 시작점(a)에서 개구부 가운데까지 (mm)
    pub offset: f64,
    /// 폭 (mm)
    pub width: f64,
    /// 높이 (mm)
    pub height: f64,
    /// 창대 높이 (창, mm)
    #[serde(default)]
    pub sill: f64,
    /// 문 여는 방향을 벽 반대쪽으로
    #[serde(default)]
    pub flip: bool,
    /// 문 경첩을 반대 끝으로
    #[serde(default)]
    pub hinge_end: bool,
    /// 창호 기호 (D1, W1 …). 비면 자동
    #[serde(default)]
    pub mark: String,
}

/// 실 (닫힌 경계, 이름)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Room {
    pub id: u64,
    pub level: u64,
    pub name: String,
    pub boundary: Vec<Vec2>,
    /// 바닥·벽·천장 마감 (실별 마감표)
    #[serde(default)]
    pub finish_floor: String,
    #[serde(default)]
    pub finish_wall: String,
    #[serde(default)]
    pub finish_ceiling: String,
}

impl Room {
    /// 면적 (도면 단위²)
    pub fn area(&self) -> f64 {
        let r = &self.boundary;
        let n = r.len();
        ((0..n)
            .map(|i| r[i].x * r[(i + 1) % n].y - r[(i + 1) % n].x * r[i].y)
            .sum::<f64>()
            / 2.0)
            .abs()
    }

    /// 무게중심 (이름 표시 위치)
    pub fn centroid(&self) -> Vec2 {
        let r = &self.boundary;
        let n = r.len();
        let (mut cx, mut cy, mut a2) = (0.0, 0.0, 0.0);
        for i in 0..n {
            let (p, q) = (r[i], r[(i + 1) % n]);
            let c = p.x * q.y - q.x * p.y;
            a2 += c;
            cx += (p.x + q.x) * c;
            cy += (p.y + q.y) * c;
        }
        if a2.abs() < 1e-12 {
            let k = n.max(1) as f64;
            return Vec2::new(
                r.iter().map(|p| p.x).sum::<f64>() / k,
                r.iter().map(|p| p.y).sum::<f64>() / k,
            );
        }
        Vec2::new(cx / (3.0 * a2), cy / (3.0 * a2))
    }
}

/// 그리드 (통줄) 한 줄
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub id: u64,
    /// X1, Y1 …
    pub label: String,
    pub a: Vec2,
    pub b: Vec2,
}

/// 대지 (법규 검토)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Site {
    /// 대지 경계
    pub boundary: Vec<Vec2>,
    /// 용도지역 이름 (참고)
    #[serde(default)]
    pub zone: String,
    /// 건폐율 한도 (%)
    #[serde(default)]
    pub coverage_limit: f64,
    /// 용적률 한도 (%)
    #[serde(default)]
    pub far_limit: f64,
}

/// 치수 (정렬 치수 — 두 점과 치수선까지 거리. 값은 두 점에서 늘 새로 셈, 구현 노트 §28)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dim {
    pub id: u64,
    pub level: u64,
    pub a: Vec2,
    pub b: Vec2,
    /// a→b 왼쪽으로 치수선까지 거리 (음수면 오른쪽)
    pub offset: f64,
    /// 글자 높이 (도면 단위)
    pub text_height: f64,
    /// 비우면 길이, 아니면 이 글자 (「<>」 는 길이로 바뀜)
    #[serde(default)]
    pub text: String,
}

impl Dim {
    pub fn length(&self) -> f64 {
        self.a.dist(self.b)
    }
}

/// 기둥 (사각, 가운데·폭·깊이·회전)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub id: u64,
    pub level: u64,
    pub at: Vec2,
    pub width: f64,
    pub depth: f64,
    #[serde(default)]
    pub angle: f64,
}

impl Column {
    /// 네 모서리 (반시계)
    pub fn corners(&self) -> [Vec2; 4] {
        let (s, c) = self.angle.sin_cos();
        let (w, d) = (self.width / 2.0, self.depth / 2.0);
        let p = |x: f64, y: f64| Vec2::new(self.at.x + x * c - y * s, self.at.y + x * s + y * c);
        [p(-w, -d), p(w, -d), p(w, d), p(-w, d)]
    }
}

/// 곧은 계단 (a 에서 b 로 올라감)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stair {
    pub id: u64,
    pub level: u64,
    pub a: Vec2,
    pub b: Vec2,
    pub width: f64,
    /// 단 수 (챌면 수)
    pub risers: u32,
}

/// 건축 객체 전체
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchModel {
    pub levels: Vec<Level>,
    pub walls: Vec<Wall>,
    pub openings: Vec<Opening>,
    pub rooms: Vec<Room>,
    pub grids: Vec<Grid>,
    pub site: Site,
    pub dims: Vec<Dim>,
    pub columns: Vec<Column>,
    pub stairs: Vec<Stair>,
    /// 지금 그리는 층
    pub active_level: u64,
    /// 다음 객체 번호
    pub next_id: u64,
}

impl ArchModel {
    /// 새 번호
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// 층이 없으면 「1층」 을 만들고, 지금 층 번호를 돌려준다
    pub fn ensure_level(&mut self) -> u64 {
        if self.levels.is_empty() {
            let id = self.new_id();
            self.levels.push(Level {
                id,
                name: "1층".into(),
                elevation: 0.0,
                height: 3000.0,
            });
            self.active_level = id;
        }
        if !self.levels.iter().any(|l| l.id == self.active_level) {
            self.active_level = self.levels[0].id;
        }
        self.active_level
    }

    pub fn level(&self, id: u64) -> Option<&Level> {
        self.levels.iter().find(|l| l.id == id)
    }

    pub fn wall(&self, id: u64) -> Option<&Wall> {
        self.walls.iter().find(|w| w.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.walls.is_empty()
            && self.rooms.is_empty()
            && self.grids.is_empty()
            && self.site.boundary.is_empty()
            && self.dims.is_empty()
            && self.columns.is_empty()
            && self.stairs.is_empty()
    }
}
