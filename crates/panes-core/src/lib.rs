//! Platform-neutral window commands, geometry, layout, and history.
//!
//! # Coordinate system
//!
//! Every [`Point`] and [`Rect`] uses one logical desktop coordinate space:
//! the primary display's lower-left corner is the origin, x increases to the
//! right, and y increases upward. Displays arranged left of or below the
//! primary display therefore have negative coordinates. Core geometry is
//! unit-agnostic; native adapters convert platform coordinates into consistent
//! logical units before calling this crate and convert calculated rectangles
//! back before applying them.
//!
//! A top-left, y-down native rectangle can be flipped around the primary
//! display's top edge and round-tripped as follows:
//!
//! ```
//! use panes_core::Rect;
//!
//! let primary_height = 900.0;
//! let native = Rect::new(100.0, 80.0, 400.0, 300.0);
//! let panes = Rect::new(
//!     native.origin.x,
//!     primary_height - native.origin.y - native.size.height,
//!     native.size.width,
//!     native.size.height,
//! );
//! assert_eq!(panes, Rect::new(100.0, 520.0, 400.0, 300.0));
//!
//! let native_again = Rect::new(
//!     panes.origin.x,
//!     primary_height - panes.origin.y - panes.size.height,
//!     panes.size.width,
//!     panes.size.height,
//! );
//! assert_eq!(native_again, native);
//! ```

pub mod command;
pub mod config;
pub mod geometry;
pub mod history;
pub mod layout;

pub use command::{Command, CommandCategory};
pub use config::LayoutConfig;
pub use geometry::{Edge, Orientation, Point, Rect, Size};
pub use history::{MAX_TRACKED_WINDOWS, RecordedCommand, WindowHistory, WindowId};
pub use layout::{LayoutRequest, LayoutResult, calculate};
