use std::process::ExitCode;

fn main() -> ExitCode {
    bonk_cli::run(std::env::args_os(), |view| {
        #[cfg(feature = "gui")]
        {
            bonk_gui::run(
                bonk_gui::ViewOptions {
                    show_internal: view.show_internal,
                    allow_virtual: view.allow_virtual,
                    show_all: view.show_all,
                },
                view.lang,
            )
        }
        #[cfg(not(feature = "gui"))]
        {
            let _ = view;
            Err("this version is command line only, without a window".to_owned())
        }
    })
}
