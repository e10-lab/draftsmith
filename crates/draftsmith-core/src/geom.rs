//! 2D 기하 기본형. 좌표는 모두 f64 (도면 단위, 보통 mm).

/// 2D 점·벡터
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
