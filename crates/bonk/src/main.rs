use std::process::ExitCode;

#[cfg(feature = "gui")]
fn window(request: bonk_cli::WindowRequest) -> Result<(), String> {
    let view = request.view;
    bonk_gui::run(
        bonk_gui::ViewOptions {
            show_internal: view.show_internal,
            allow_virtual: view.allow_virtual,
            show_all: view.show_all,
            offline: view.offline,
            incognito: view.incognito,
            notify: view.notify,
        },
        view.lang,
        bonk_gui::ThemeChoice::from_config(Some(view.theme)),
        request.image,
    )
}

fn main() -> ExitCode {
    #[cfg(feature = "gui")]
    let window = Some(window);
    #[cfg(not(feature = "gui"))]
    let window: Option<fn(bonk_cli::WindowRequest) -> Result<(), String>> = None;
    bonk_cli::run(std::env::args_os(), window)
}
