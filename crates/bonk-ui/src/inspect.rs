use bonk_core::pe::Signer;
use serde_json::Value;

use crate::model::{Effect, Effects, Model, WriteState};
use crate::text::secure_boot_sentence;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub image: String,
    pub facts: Option<Facts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub refusal: Option<String>,
    pub signer: Option<Signer>,
}

#[must_use]
pub fn signer_of(report: &Value) -> Option<Signer> {
    serde_json::from_value(report["extraction"]["signer"].clone())
        .or_else(|_| serde_json::from_value(report["efi_loaders"][0]["signer"].clone()))
        .ok()
}

// An address is read only once downloaded.
fn is_local(image: &str) -> bool {
    !image.is_empty() && !image.contains("://")
}

impl Model {
    pub(crate) fn inspect_draft(&mut self) -> Effects {
        let WriteState::Draft { image, .. } = &self.write else {
            return Vec::new();
        };
        let image = image.trim();
        if !is_local(image)
            || self
                .inspection
                .as_ref()
                .is_some_and(|known| known.image == image)
        {
            return Vec::new();
        }
        self.inspection = Some(Inspection {
            image: image.to_owned(),
            facts: None,
        });
        vec![Effect::InspectImage(image.to_owned())]
    }

    pub(crate) fn on_inspected(&mut self, image: &str, report: Option<&Value>) {
        if let Some(inspection) = self
            .inspection
            .as_mut()
            .filter(|known| known.image == image)
        {
            inspection.facts = report.map(|report| Facts {
                refusal: report["refusal"].as_str().map(str::to_owned),
                signer: signer_of(report),
            });
        }
    }

    #[must_use]
    pub fn boot_line(&self) -> Option<String> {
        let WriteState::Draft { image, raw, .. } = &self.write else {
            return None;
        };
        let facts = self
            .inspection
            .as_ref()
            .filter(|known| known.image == image.trim())?
            .facts
            .as_ref()?;
        match (&facts.refusal, &facts.signer) {
            (Some(reason), _) if !raw => Some(format!(
                "{} {reason}",
                self.lang
                    .pick("Bonk refuses this image:", "Bonk refuse cette image :")
            )),
            (_, Some(signer)) => Some(secure_boot_sentence(self.lang, signer)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
