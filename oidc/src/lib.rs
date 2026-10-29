mod browser;
mod token;

pub use browser::{
    AuthorizationPage, BrowserSignIn, BrowserSignInSettings, PendingSignIn, SignInHangUp,
};
pub use token::{TokenCheckSettings, TokenVerifier};

/// The prefix of the provider's settings in the environment.
pub const ENV_PREFIX: &str = "RIFT_AUTH_";
