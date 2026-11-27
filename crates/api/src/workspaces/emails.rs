use crate::mail::OutgoingEmail;

/// The invitation. The workspace's name and the inviter's are text a person typed, kept free of control
/// characters (so nothing can be added to the subject or the body's layout).
pub fn invitation(
    to: &str,
    workspace: &str,
    inviter: &str,
    link: &str,
    days: i64,
) -> OutgoingEmail {
    OutgoingEmail {
        to: to.to_owned(),
        subject: format!("{inviter} invited you to {workspace} on Zeno"),
        text: format!(
            "{inviter} invited you to join the workspace \"{workspace}\" on Zeno.\n\nAccept the invitation (you'll sign in, or make an account with this address, first):\n{link}\n\nThe link works for {days} days, and only for the account with this email address. If you don't know {inviter}, or weren't expecting this, you can ignore it: nothing happens unless you accept.\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_who_asked_what_for_and_where_the_link_goes() {
        let mail = invitation(
            "a@x.in",
            "Maths 9A",
            "Asha Rao",
            "https://zeno.test/invite/T",
            7,
        );
        assert_eq!(mail.to, "a@x.in");
        assert_eq!(mail.subject, "Asha Rao invited you to Maths 9A on Zeno");
        assert!(mail.text.contains("https://zeno.test/invite/T"));
        assert!(mail.text.contains("7 days"));
        assert!(mail
            .text
            .contains("only for the account with this email address"));
    }
}
