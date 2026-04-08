//! `.inkplate` project file format.
//!
//! A project file is a small JSON document that captures everything the
//! GUI needs to rehydrate a session: source image path, shirt color,
//! global job settings, workflow + workflow opts, and the full layer
//! list with all tweaks. Cache fields (preview masks, processed
//! rasters) are rebuilt on load — they're never persisted.
//!
//! The `version` field is at the top level so we can migrate old
//! projects forward without losing work.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::engine::color::Rgb;
use crate::engine::layer::Layer;
use crate::engine::pipeline::JobOpts;
use crate::engine::workflows::{Workflow, WorkflowOpts};

/// Current schema version. Bump when anything incompatible changes.
pub const CURRENT_VERSION: u32 = 1;

/// Full project snapshot — the contents of a `.inkplate` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub source_path: Option<PathBuf>,
    pub shirt_color: Rgb,
    #[serde(with = "job_opts_serde")]
    pub job: JobOpts,
    pub workflow: Workflow,
    pub workflow_opts: WorkflowOpts,
    pub layers: Vec<Layer>,
}

impl Project {
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self).context("serialize project to JSON")?;
        std::fs::write(path, json)
            .with_context(|| format!("write project to {}", path.display()))?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes =
            std::fs::read(path).with_context(|| format!("read project from {}", path.display()))?;
        let mut project: Project = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse project JSON: {}", path.display()))?;
        if project.version > CURRENT_VERSION {
            anyhow::bail!(
                "project was saved with a newer version ({}); this build supports up to {}",
                project.version,
                CURRENT_VERSION
            );
        }
        migrate(&mut project)?;
        Ok(project)
    }
}

/// Migrate an older project schema in place. Each bump appends a step.
fn migrate(_project: &mut Project) -> anyhow::Result<()> {
    // TODO(L6): if we ever bump CURRENT_VERSION, add migration steps
    // here keyed on the old value. For now, v1 is the only version so
    // there's nothing to migrate.
    Ok(())
}

// `JobOpts` isn't serde-derivable directly because it lives in the
// engine module and we don't want to pollute the engine with a serde
// dep requirement. A tiny newtype wrapper keeps the concern here.
mod job_opts_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::engine::pipeline::JobOpts;

    #[derive(Serialize, Deserialize)]
    struct JobOptsRepr {
        dpi: u32,
        default_lpi: f32,
        default_angle_deg: f32,
    }

    pub fn serialize<S: Serializer>(value: &JobOpts, ser: S) -> Result<S::Ok, S::Error> {
        JobOptsRepr {
            dpi: value.dpi,
            default_lpi: value.default_lpi,
            default_angle_deg: value.default_angle_deg,
        }
        .serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<JobOpts, D::Error> {
        let r = JobOptsRepr::deserialize(de)?;
        Ok(JobOpts {
            dpi: r.dpi,
            default_lpi: r.default_lpi,
            default_angle_deg: r.default_angle_deg,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::color::Rgb;
    use crate::engine::layer::Layer;

    #[test]
    fn roundtrip_empty_project() {
        let project = Project {
            version: CURRENT_VERSION,
            source_path: None,
            shirt_color: Rgb(20, 20, 30),
            job: JobOpts::default(),
            workflow: Workflow::SimprocessDark,
            workflow_opts: WorkflowOpts::default(),
            layers: vec![Layer::new_spot(Rgb(255, 0, 0))],
        };
        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, CURRENT_VERSION);
        assert_eq!(back.layers.len(), 1);
        assert_eq!(back.shirt_color.0, 20);
    }
}
