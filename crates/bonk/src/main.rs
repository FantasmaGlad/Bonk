use std::process::ExitCode;

fn main() -> ExitCode {
    bonk_cli::run(std::env::args_os(), |request| {
        #[cfg(feature = "gui")]
        {
            let view = request.view;
            bonk_gui::run(
                bonk_gui::ViewOptions {
                    show_internal: view.show_internal,
                    allow_virtual: view.allow_virtual,
                    show_all: view.show_all,
                },
                view.lang,
                bonk_gui::ThemeChoice::from_config(Some(view.theme)),
                request.image,
            )
        }
        #[cfg(not(feature = "gui"))]
        {
            let _ = request;
            Err("this version is command line only, without a window".to_owned())
        }
    })
}
