use crate::mail::OutgoingEmail;

pub fn verification(to: &str, link: &str) -> OutgoingEmail {
    OutgoingEmail {
        to: to.to_owned(),
        subject: "Verify your Zeno account".into(),
        text: format!(
            "Welcome to Zeno!\n\nConfirm your email address to finish creating your account:\n{link}\n\nThis link works once and expires in 24 hours. If you didn't sign up, you can ignore this email.\n"
        ),
    }
}

pub fn already_registered(to: &str, sign_in_link: &str, reset_link: &str) -> OutgoingEmail {
    OutgoingEmail {
        to: to.to_owned(),
        subject: "You already have a Zeno account".into(),
        text: format!(
            "Someone tried to create a Zeno account with this email address, but you already have one.\n\nSign in: {sign_in_link}\nForgot your password? {reset_link}\n\nIf this wasn't you, no action is needed.\n"
        ),
    }
}

pub fn password_reset(to: &str, link: &str) -> OutgoingEmail {
    OutgoingEmail {
        to: to.to_owned(),
        subject: "Reset your Zeno password".into(),
        text: format!(
            "We received a request to reset your password:\n{link}\n\nThis link works once and expires in 1 hour. If you didn't ask for this, you can ignore this email; your password is unchanged.\n"
        ),
    }
}

pub fn password_changed(to: &str) -> OutgoingEmail {
    OutgoingEmail {
        to: to.to_owned(),
        subject: "Your Zeno password was changed".into(),
        text: "Your password was just changed and you've been signed out everywhere.\n\nIf this wasn't you, reset your password immediately and contact support.\n".into(),
    }
}
