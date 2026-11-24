//! DraftSmith 코어.
//!
//! - [`drawing`]: 도면 모델 (레이어·작도 객체). 앞으로 건축 객체(벽·문·창·실)가 여기에 더해진다.
//! - [`import`]: DXF·DWG 를 읽어 도면 모델로 바꾼다 (acadrust).
//! - [`display`]: 도면을 화면(WebGL2)에 넘길 바이너리 표시 데이터로 만든다.
//! - [`sample`]: M0 성능 측정용 도면을 만든다.
//!
//! 화면(앱)과 CLI 는 이 crate 만 쓴다. 화면 관련 코드는 여기에 두지 않는다.

pub mod curve;
pub mod display;
pub mod drawing;
pub mod fill;
pub mod geom;
pub mod import;
pub mod sample;
pub mod text;

pub use drawing::{Drawing, Entity, Layer, Shape, TextShape, VAlign};
pub use geom::{Affine, Bbox, Vec2};
