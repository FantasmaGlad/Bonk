use std::io::ErrorKind;

use super::authopen_error;

#[test]
fn authopen_dit_pourquoi_le_disque_n_a_pas_ete_ouvert() {
    let cases = [
        (
            Some(1),
            "open: Resource busy",
            ErrorKind::ResourceBusy,
            "authopen: open: Resource busy",
        ),
        (
            Some(1),
            "",
            ErrorKind::PermissionDenied,
            "authorization refused or cancelled (authopen exit status 1)",
        ),
        (
            None,
            "",
            ErrorKind::PermissionDenied,
            "authorization refused or cancelled",
        ),
    ];
    for (status, said, kind, message) in cases {
        let error = authopen_error(status, said);
        assert_eq!(
            (error.kind(), error.to_string()),
            (kind, message.to_owned())
        );
    }
}
