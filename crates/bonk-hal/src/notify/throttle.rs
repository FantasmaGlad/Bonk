use std::time::{Duration, Instant};

use super::Notice;

const PACE: Duration = Duration::from_secs(1);

/// At most one notice a second, except a new step or the end.
#[derive(Debug, Default)]
pub struct Throttle {
    previous: Option<(String, Instant)>,
}

impl Throttle {
    pub fn admit(&mut self, notice: &Notice, now: Instant) -> bool {
        let admitted = notice.last
            || self.previous.as_ref().is_none_or(|(title, at)| {
                *title != notice.title || now.duration_since(*at) >= PACE
            });
        if admitted {
            self.previous = (!notice.last).then(|| (notice.title.clone(), now));
        }
        admitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(title: &str, last: bool) -> Notice {
        Notice {
            title: title.to_owned(),
            body: String::new(),
            percent: None,
            last,
        }
    }

    #[test]
    fn une_seule_notification_par_seconde_sauf_changement_d_etape() {
        let start = Instant::now();
        let mut throttle = Throttle::default();
        assert!(throttle.admit(&notice("Écriture", false), start));
        assert!(!throttle.admit(
            &notice("Écriture", false),
            start + Duration::from_millis(400)
        ));
        assert!(throttle.admit(
            &notice("Relecture", false),
            start + Duration::from_millis(500)
        ));
        assert!(throttle.admit(
            &notice("Relecture", false),
            start + Duration::from_millis(1500)
        ));
    }

    #[test]
    fn la_fin_passe_toujours_et_la_suivante_repart_de_zero() {
        let start = Instant::now();
        let mut throttle = Throttle::default();
        assert!(throttle.admit(&notice("Écriture", false), start));
        assert!(throttle.admit(&notice("Clé prête", true), start));
        assert!(throttle.admit(&notice("Écriture", false), start));
    }
}
