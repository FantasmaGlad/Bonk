use iced::futures::channel::oneshot;

/// Blocking work on its own thread. The window's executor has a single thread (`smol` by default): a cold
/// catalog read held every other task behind it, the library list included. `None` if the work panicked.
pub(super) fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Option<T>> {
    let (sender, receiver) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    async move { receiver.await.ok() }
}

pub(super) fn off_thread_result<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    let done = off_thread(work);
    async move {
        done.await
            .unwrap_or_else(|| Err("the work stopped unexpectedly".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use iced::futures::executor::block_on;
    use iced::futures::future::join;

    use super::*;

    #[test]
    fn deux_travaux_bloquants_ne_s_attendent_pas() {
        let slow = || {
            off_thread(|| {
                std::thread::sleep(Duration::from_millis(300));
                1
            })
        };
        let started = Instant::now();
        let (a, b) = block_on(join(slow(), slow()));
        assert_eq!((a, b), (Some(1), Some(1)));
        assert!(
            started.elapsed() < Duration::from_millis(550),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn un_travail_qui_panique_rend_une_erreur() {
        let done = block_on(off_thread_result::<()>(|| panic!("panique simulée")));
        assert!(done.is_err());
    }
}
