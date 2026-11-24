//! 2D 기하 기본형. 좌표는 모두 f64 (도면 단위, 보통 mm).

/// 2D 점·벡터
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dist(self, o: Vec2) -> f64 {
        (self.x - o.x).hypot(self.y - o.y)
    }

    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        Vec2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
}

/// 축 정렬 경계 상자. 비어 있으면 `min > max`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bbox {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for Bbox {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Bbox {
    pub const EMPTY: Bbox = Bbox {
        min: Vec2::new(f64::INFINITY, f64::INFINITY),
        max: Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    };

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    /// 점 하나를 넣는다. NaN·무한대 좌표는 무시한다 (깨진 도면에서 범위가 망가지지 않게).
    pub fn add(&mut self, p: Vec2) {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn merge(&mut self, o: &Bbox) {
        if !o.is_empty() {
            self.add(o.min);
            self.add(o.max);
        }
    }

    pub fn center(&self) -> Vec2 {
        if self.is_empty() {
            Vec2::default()
        } else {
            self.min.lerp(self.max, 0.5)
        }
    }

    pub fn width(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            self.max.x - self.min.x
        }
    }

    pub fn height(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            self.max.y - self.min.y
        }
    }
}

/// 2D 아핀 변환: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
/// 블록 참조(INSERT)의 이동·회전·축척·대칭을 담는다.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(x: f64, y: f64) -> Self {
        Affine {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub fn rotate(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Affine {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn scale(sx: f64, sy: f64) -> Self {
        Affine {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// `self` 를 먼저, 그다음 `then` 을 적용하는 변환
    pub fn then(&self, then: &Affine) -> Affine {
        let t = then;
        Affine {
            a: t.a * self.a + t.c * self.b,
            b: t.b * self.a + t.d * self.b,
            c: t.a * self.c + t.c * self.d,
            d: t.b * self.c + t.d * self.d,
            e: t.a * self.e + t.c * self.f + t.e,
            f: t.b * self.e + t.d * self.f + t.f,
        }
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    /// 방향 벡터 (이동 빼고)
    pub fn linear(&self, v: Vec2) -> Vec2 {
        Vec2::new(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }

    pub fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// 대칭(거울상)이 들어 있으면 true — 호의 방향이 뒤집힌다
    pub fn mirrors(&self) -> bool {
        self.det() < 0.0
    }

    /// x 축 방향 확대율
    pub fn scale_x(&self) -> f64 {
        self.a.hypot(self.b)
    }

    /// 원이 원으로 남는 변환인가 (회전·균등 축척·이동만, 대칭 없음)
    pub fn is_conformal(&self) -> bool {
        let tol = 1e-9 * (self.a.abs() + self.b.abs() + self.c.abs() + self.d.abs()).max(1e-300);
        (self.a - self.d).abs() <= tol && (self.b + self.c).abs() <= tol
    }

    /// 대칭이 든 등각 변환인가 (대칭 + 회전·균등 축척) — 원은 원으로 남고 방향만 뒤집힌다
    pub fn is_anticonformal(&self) -> bool {
        let tol = 1e-9 * (self.a.abs() + self.b.abs() + self.c.abs() + self.d.abs()).max(1e-300);
        (self.a + self.d).abs() <= tol && (self.b - self.c).abs() <= tol
    }

    /// x 축이 향하는 각도
    pub fn angle(&self) -> f64 {
        self.b.atan2(self.a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_compose_order() {
        // 먼저 (1,0) 이동, 그다음 90° 회전 → (0,1)
        let t = Affine::translate(1.0, 0.0).then(&Affine::rotate(std::f64::consts::FRAC_PI_2));
        let p = t.apply(Vec2::default());
        assert!(
            (p.x - 0.0).abs() < 1e-12 && (p.y - 1.0).abs() < 1e-12,
            "{p:?}"
        );
        assert!(
            Affine::rotate(0.3)
                .then(&Affine::scale(2.0, 2.0))
                .is_conformal()
        );
        assert!(!Affine::scale(-1.0, 1.0).is_conformal());
        assert!(Affine::scale(-1.0, 1.0).mirrors());
    }

    #[test]
    fn bbox_ignores_non_finite() {
        let mut b = Bbox::EMPTY;
        assert!(b.is_empty());
        b.add(Vec2::new(1.0, 2.0));
        b.add(Vec2::new(f64::NAN, 5.0));
        b.add(Vec2::new(-3.0, 4.0));
        assert_eq!(b.min, Vec2::new(-3.0, 2.0));
        assert_eq!(b.max, Vec2::new(1.0, 4.0));
        assert_eq!(b.center(), Vec2::new(-1.0, 3.0));
    }
}
