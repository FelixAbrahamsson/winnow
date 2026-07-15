//! winnow-core: pure (non-GUI) logic for the winnow image culling tool —
//! folder scanning, metadata, bucket config, and the reversible move/undo
//! session model. Kept free of any GUI dependency so it builds and tests
//! standalone.

pub mod buckets;
pub mod metadata;
pub mod model;
pub mod scan;

pub use buckets::{Bucket, CONFIG_NAME};
pub use metadata::{Metadata, SortKey};
pub use model::{ImageItem, Session};
pub use scan::{is_image, scan_folder, IMAGE_EXTENSIONS};
