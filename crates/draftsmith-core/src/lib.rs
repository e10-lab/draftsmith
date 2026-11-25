//! DraftSmith 코어.
//!
//! - [`drawing`]: 도면 모델 (레이어·작도 객체). 앞으로 건축 객체(벽·문·창·실)가 여기에 더해진다.
//! - [`import`]: DXF·DWG 를 읽어 도면 모델로 바꾼다 (acadrust).
//! - [`display`]: 도면을 화면(WebGL2)에 넘길 바이너리 표시 데이터로 만든다.
//! - [`sample`]: M0 성능 측정용 도면을 만든다.
//!
//! 화면(앱)과 CLI 는 이 crate 만 쓴다. 화면 관련 코드는 여기에 두지 않는다.

pub mod arch;
pub mod archgen;
pub mod block;
pub mod ctb;
pub mod curve;
pub mod demo;
pub mod display;
pub mod docfile;
pub mod drawing;
pub mod edit;
pub mod export;
pub mod fill;
pub mod geom;
pub mod grip;
pub mod ifc;
pub mod ifc_read;
pub mod import;
pub mod layout;
pub mod model3d;
pub mod modify;
pub mod pdf;
pub mod pick;
pub mod sample;
pub mod section;
pub mod sheet;
pub mod snap;
pub mod text;

pub use drawing::{Drawing, Entity, Layer, LinePattern, LineStyle, Shape, TextShape, VAlign};
pub use geom::{Affine, Bbox, Vec2};
