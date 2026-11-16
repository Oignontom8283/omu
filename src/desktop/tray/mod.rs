//! The icon in the notification area (the tray): a click opens the interface, the menu stops the engine or quits.
//!
//! It is carried by the daemon, which already runs in the user's session, so the icon exists as soon as the daemon does,
//! with no window of its own and no extra executable. It lives on its own thread and a failure there never touches the
//! audio: if the icon cannot be made, the daemon says so in its log and goes on.
//!
//! Only Windows has an icon for now; elsewhere [`Tray::start`] says so and nothing happens.

/// What the icon does. The functions are called on the icon's own thread: they start the work and return at once.
pub struct Actions {
    /// A click on the icon, or "Open the interface".
    pub open: Box<dyn Fn() + Send + Sync>,
    /// "Start the engine": the graph that was last applied runs again.
    pub start_engine: Box<dyn Fn() + Send + Sync>,
    /// "Stop the engine": the audio stops, the daemon stays.
    pub stop_engine: Box<dyn Fn() + Send + Sync>,
    /// "Quit": the daemon ends.
    pub quit: Box<dyn Fn() + Send + Sync>,
}

/// The texts of the menu, in the language of the computer (English and French for now).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texts {
    pub open: &'static str,
    pub start_engine: &'static str,
    pub stop_engine: &'static str,
    pub quit: &'static str,
}

impl Texts {
    /// The texts for a locale name such as `fr-FR` or `en-US` (English when the language is not one of ours).
    pub fn for_locale(locale: &str) -> Texts {
        if locale.to_ascii_lowercase().starts_with("fr") {
            Texts {
                open: "Ouvrir l’interface",
                start_engine: "Démarrer le moteur",
                stop_engine: "Arrêter le moteur",
                quit: "Quitter",
            }
        } else {
            Texts {
                open: "Open the interface",
                start_engine: "Start the engine",
                stop_engine: "Stop the engine",
                quit: "Quit",
            }
        }
    }
}

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use self::win::Tray;

/// The icon where there is none yet.
#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn start(_actions: Actions) -> Result<Tray, String> {
        Err("the notification-area icon is not available on this system yet".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_speaks_the_language_of_the_computer() {
        assert_eq!(Texts::for_locale("fr-FR").quit, "Quitter");
        assert_eq!(Texts::for_locale("FR").open, "Ouvrir l’interface");
        assert_eq!(Texts::for_locale("en-US").quit, "Quit");
        assert_eq!(Texts::for_locale("de-DE").stop_engine, "Stop the engine");
        assert_eq!(Texts::for_locale("").open, "Open the interface");
    }
}
