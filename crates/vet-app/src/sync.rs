//! Platform sync (§11). Branches on `PlatformTraits`, never on a platform's name.
//! Local review state (spans) is never synced; threads sync both ways.

use vet_core::platform::{PlatformTraits, Posting};

use crate::ports::Platform;

/// What the toolbar offers, derived from the platform's traits.
pub fn submit_label(traits: &PlatformTraits) -> &'static str {
    match traits.posting {
        Posting::BatchedReview => "Submit review",
        Posting::PerComment => "Post",
    }
}

pub fn toolbar_label(platform: &impl Platform) -> &'static str {
    submit_label(&platform.traits())
}
