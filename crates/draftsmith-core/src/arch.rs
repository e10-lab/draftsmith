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
    /// 끝점을 묶은 건축 객체 (벽·그리드·기둥) — 그 객체가 움직이면 따라간다 (§28)
    #[serde(default)]
    pub ka: Option<Anchor>,
    #[serde(default)]
    pub kb: Option<Anchor>,
    /// 회전 치수의 방향 (라디안 — 0 가로, π/2 세로). None 이면 정렬 치수 (두 점 방향)
    #[serde(default)]
    pub rotation: Option<f64>,
    /// 레이어 이름 (비면 A-ANNO-DIMS)
    #[serde(default)]
    pub layer: String,
}

/// 치수의 지금 모양: 두 점, 치수 방향 u·왼쪽 n, 치수선 위 두 점, 값
#[derive(Debug, Clone, Copy)]
pub struct DimGeom {
    pub a: Vec2,
    pub b: Vec2,
    pub u: Vec2,
    pub n: Vec2,
    pub da: Vec2,
    pub db: Vec2,
    pub len: f64,
}

/// 건축 객체에 묶은 점: 그 객체 좌표계(원점·방향)에서 (t 앞, s 왼쪽)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub what: String,
    pub id: u64,
    pub t: f64,
    pub s: f64,
}

impl Dim {
    /// 저장된 두 점 길이 (묶인 점은 [`Dim::ends`] 로)
    pub fn length(&self) -> f64 {
        self.a.dist(self.b)
    }

    /// 지금 모양. 치수선은 a 에서 n 쪽으로 offset 떨어진 곳, u 방향 (정렬이면 a→b 방향)
    pub fn geom(&self, m: &ArchModel) -> Option<DimGeom> {
        let (a, b) = self.ends(m);
        let u = match self.rotation {
            Some(t) => Vec2::new(t.cos(), t.sin()),
            None => {
                let l = a.dist(b);
                if l < 1e-9 {
                    return None;
                }
                Vec2::new((b.x - a.x) / l, (b.y - a.y) / l)
            }
        };
        let n = Vec2::new(-u.y, u.x);
        let dot = |p: Vec2, q: Vec2| p.x * q.x + p.y * q.y;
        let ab = Vec2::new(b.x - a.x, b.y - a.y);
        let len = dot(ab, u).abs();
        if len < 1e-9 {
            return None;
        }
        let da = Vec2::new(a.x + n.x * self.offset, a.y + n.y * self.offset);
        let kb = self.offset - dot(ab, n);
        let db = Vec2::new(b.x + n.x * kb, b.y + n.y * kb);
        Some(DimGeom {
            a,
            b,
            u,
            n,
            da,
            db,
            len,
        })
    }

    /// 지금 두 끝점: 묶인 객체가 있으면 그 객체를 따라, 없으면 저장된 점
    pub fn ends(&self, m: &ArchModel) -> (Vec2, Vec2) {
        let get =
            |k: &Option<Anchor>, p: Vec2| k.as_ref().and_then(|k| m.anchor_point(k)).unwrap_or(p);
        (get(&self.ka, self.a), get(&self.kb, self.b))
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

/// 계단 (a 에서 b 로 올라감). `mid` 가 있으면 그 점(중심선 꺾이는 곳)에 참이 있는 꺾인(ㄱ자) 계단 (§34)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stair {
    pub id: u64,
    pub level: u64,
    pub a: Vec2,
    pub b: Vec2,
    pub width: f64,
    /// 단 수 (챌면 수)
    pub risers: u32,
    #[serde(default)]
    pub mid: Option<Vec2>,
}

/// 계단 한 줄기: 시작점(중심선), 방향, 길이, 단 수, 앞 줄기까지 단 수
#[derive(Debug, Clone, Copy)]
pub struct Flight {
    pub start: Vec2,
    pub u: Vec2,
    pub len: f64,
    pub risers: u32,
    pub first: u32,
}

impl Stair {
    /// 줄기들과 참(네 모서리). 꺾인 계단은 참 폭만큼 두 줄기가 짧아지고, 단 수는 길이 비례로 나눈다
    pub fn flights(&self) -> Option<(Vec<Flight>, Option<[Vec2; 4]>)> {
        let unit = |a: Vec2, b: Vec2| {
            let l = a.dist(b);
            (l > 1e-9).then(|| (Vec2::new((b.x - a.x) / l, (b.y - a.y) / l), l))
        };
        let r = self.risers.max(1);
        match self.mid {
            None => {
                let (u, len) = unit(self.a, self.b)?;
                Some((
                    vec![Flight {
                        start: self.a,
                        u,
                        len,
                        risers: r,
                        first: 0,
                    }],
                    None,
                ))
            }
            Some(c) => {
                let (u1, l1) = unit(self.a, c)?;
                let (u2, l2) = unit(c, self.b)?;
                let h = self.width / 2.0;
                let (l1, l2) = (l1 - h, l2 - h);
                if l1 <= 1e-6 || l2 <= 1e-6 || r < 2 {
                    return None;
                }
                let n1 = ((r as f64 * l1 / (l1 + l2)).round() as u32).clamp(1, r - 1);
                let p = |s: f64, t: f64| {
                    Vec2::new(
                        c.x + (u1.x * s + u2.x * t) * h,
                        c.y + (u1.y * s + u2.y * t) * h,
                    )
                };
                Some((
                    vec![
                        Flight {
                            start: self.a,
                            u: u1,
                            len: l1,
                            risers: n1,
                            first: 0,
                        },
                        Flight {
                            start: p(0.0, 1.0),
                            u: u2,
                            len: l2,
                            risers: r - n1,
                            first: n1,
                        },
                    ],
                    Some([p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0)]),
                ))
            }
        }
    }
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

    /// 객체 좌표계 (원점, 단위 방향): 벽·그리드는 a 에서 b 로, 기둥은 가운데에서 회전 방향
    pub fn frame(&self, what: &str, id: u64) -> Option<(Vec2, Vec2)> {
        let unit = |a: Vec2, b: Vec2| {
            let l = a.dist(b).max(1e-12);
            Vec2::new((b.x - a.x) / l, (b.y - a.y) / l)
        };
        match what {
            "wall" => self.wall(id).map(|w| (w.a, unit(w.a, w.b))),
            "grid" => self
                .grids
                .iter()
                .find(|g| g.id == id)
                .map(|g| (g.a, unit(g.a, g.b))),
            "column" => self
                .columns
                .iter()
                .find(|c| c.id == id)
                .map(|c| (c.at, Vec2::new(c.angle.cos(), c.angle.sin()))),
            _ => None,
        }
    }

    /// 묶인 점의 지금 자리
    pub fn anchor_point(&self, k: &Anchor) -> Option<Vec2> {
        let (o, u) = self.frame(&k.what, k.id)?;
        Some(Vec2::new(
            o.x + u.x * k.t - u.y * k.s,
            o.y + u.y * k.t + u.x * k.s,
        ))
    }

    /// 점 p 를 묶을 객체 찾기 (층 `level`, 치수 방향 `dir`). 그리드 선 위 > 기둥 안 > 벽 안 순, 같은 종류면 치수와 가장
    /// 직각인 것 (가로 치수 끝은 세로 그리드에 묶여야 그리드를 옮길 때 따라간다)
    pub fn anchor_at(&self, level: u64, p: Vec2, dir: Vec2, tol: f64) -> Option<Anchor> {
        let local = |o: Vec2, u: Vec2| {
            let (dx, dy) = (p.x - o.x, p.y - o.y);
            (dx * u.x + dy * u.y, -dx * u.y + dy * u.x)
        };
        let perp = |u: Vec2| (u.x * dir.x + u.y * dir.y).abs();
        let mut best: Option<(f64, Anchor)> = None;
        let mut offer = |rank: f64, what: &str, id: u64, o: Vec2, u: Vec2| {
            let (t, s) = local(o, u);
            let score = rank + perp(u);
            if best.as_ref().is_none_or(|(b, _)| score < *b) {
                best = Some((
                    score,
                    Anchor {
                        what: what.into(),
                        id,
                        t,
                        s,
                    },
                ));
            }
        };
        for g in &self.grids {
            let (o, u) = self.frame("grid", g.id)?;
            let (t, s) = local(o, u);
            if s.abs() <= tol && t >= -tol && t <= g.a.dist(g.b) + tol {
                offer(0.0, "grid", g.id, o, u);
            }
        }
        for c in self.columns.iter().filter(|c| c.level == level) {
            let (o, u) = (c.at, Vec2::new(c.angle.cos(), c.angle.sin()));
            let (t, s) = local(o, u);
            if t.abs() <= c.width / 2.0 + tol && s.abs() <= c.depth / 2.0 + tol {
                offer(10.0, "column", c.id, o, u);
            }
        }
        for w in self.walls.iter().filter(|w| w.level == level) {
            let (o, u) = (w.a, w.dir());
            let (t, s) = local(o, u);
            if s.abs() <= w.thickness / 2.0 + tol
                && t >= -w.thickness - tol
                && t <= w.length() + w.thickness + tol
            {
                offer(20.0, "wall", w.id, o, u);
            }
        }
        best.map(|(_, a)| a)
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
