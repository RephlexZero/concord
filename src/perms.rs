//! Discord permission bits, their names, and the friendly aliases the spec
//! accepts.
//!
//! The canonical names are Discord's own (see the "Permissions" page of the
//! Discord developer docs). Aliases exist because Discord's names don't match
//! what the client shows: on a forum, "Create Posts" is really
//! `SEND_MESSAGES` and "Send Messages in Posts" is `SEND_MESSAGES_IN_THREADS`.

use std::fmt;

/// A set of Discord permission bits.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Perms(pub u64);

impl Perms {
    pub const NONE: Perms = Perms(0);
    pub const ADMINISTRATOR: Perms = Perms(1 << 3);
    pub const MANAGE_CHANNELS: Perms = Perms(1 << 4);
    pub const VIEW_CHANNEL: Perms = Perms(1 << 10);
    pub const SEND_MESSAGES: Perms = Perms(1 << 11);
    pub const MANAGE_ROLES: Perms = Perms(1 << 28);
    pub const MANAGE_THREADS: Perms = Perms(1 << 34);

    pub const fn bits(self) -> u64 {
        self.0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub const fn contains(self, other: Perms) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Perms) -> bool {
        self.0 & other.0 != 0
    }
    pub const fn union(self, other: Perms) -> Perms {
        Perms(self.0 | other.0)
    }
    pub const fn minus(self, other: Perms) -> Perms {
        Perms(self.0 & !other.0)
    }
    pub const fn intersect(self, other: Perms) -> Perms {
        Perms(self.0 & other.0)
    }

    /// Parse Discord's wire format (a decimal string).
    pub fn from_wire(s: &str) -> Option<Perms> {
        s.parse().ok().map(Perms)
    }
}

impl fmt::Debug for Perms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Perms({})", names(*self, PermContext::Role).join("|"))
    }
}

/// Where a permission list appears. Some aliases only make sense in one place
/// (`CREATE_POSTS` is meaningless on a text channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermContext {
    Role,
    Category,
    Text,
    Forum,
    Voice,
}

/// Discord's permission names and bit positions.
pub const TABLE: &[(&str, u32)] = &[
    ("CREATE_INSTANT_INVITE", 0),
    ("KICK_MEMBERS", 1),
    ("BAN_MEMBERS", 2),
    ("ADMINISTRATOR", 3),
    ("MANAGE_CHANNELS", 4),
    ("MANAGE_GUILD", 5),
    ("ADD_REACTIONS", 6),
    ("VIEW_AUDIT_LOG", 7),
    ("PRIORITY_SPEAKER", 8),
    ("STREAM", 9),
    ("VIEW_CHANNEL", 10),
    ("SEND_MESSAGES", 11),
    ("SEND_TTS_MESSAGES", 12),
    ("MANAGE_MESSAGES", 13),
    ("EMBED_LINKS", 14),
    ("ATTACH_FILES", 15),
    ("READ_MESSAGE_HISTORY", 16),
    ("MENTION_EVERYONE", 17),
    ("USE_EXTERNAL_EMOJIS", 18),
    ("VIEW_GUILD_INSIGHTS", 19),
    ("CONNECT", 20),
    ("SPEAK", 21),
    ("MUTE_MEMBERS", 22),
    ("DEAFEN_MEMBERS", 23),
    ("MOVE_MEMBERS", 24),
    ("USE_VAD", 25),
    ("CHANGE_NICKNAME", 26),
    ("MANAGE_NICKNAMES", 27),
    ("MANAGE_ROLES", 28),
    ("MANAGE_WEBHOOKS", 29),
    ("MANAGE_GUILD_EXPRESSIONS", 30),
    ("USE_APPLICATION_COMMANDS", 31),
    ("REQUEST_TO_SPEAK", 32),
    ("MANAGE_EVENTS", 33),
    ("MANAGE_THREADS", 34),
    ("CREATE_PUBLIC_THREADS", 35),
    ("CREATE_PRIVATE_THREADS", 36),
    ("USE_EXTERNAL_STICKERS", 37),
    ("SEND_MESSAGES_IN_THREADS", 38),
    ("USE_EMBEDDED_ACTIVITIES", 39),
    ("MODERATE_MEMBERS", 40),
    ("VIEW_CREATOR_MONETIZATION_ANALYTICS", 41),
    ("USE_SOUNDBOARD", 42),
    ("CREATE_GUILD_EXPRESSIONS", 43),
    ("CREATE_EVENTS", 44),
    ("USE_EXTERNAL_SOUNDS", 45),
    ("SEND_VOICE_MESSAGES", 46),
    // Clyde was retired in 2023; the bit is still set on older servers.
    ("USE_CLYDE_AI", 47),
    ("SET_VOICE_CHANNEL_STATUS", 48),
    ("SEND_POLLS", 49),
    ("USE_EXTERNAL_APPS", 50),
    ("PIN_MESSAGES", 51),
    ("BYPASS_SLOWMODE", 52),
];

/// Friendly names accepted in the spec: (alias, Discord name, only valid in).
pub const ALIASES: &[(&str, &str, Option<PermContext>)] = &[
    ("CREATE_POSTS", "SEND_MESSAGES", Some(PermContext::Forum)),
    ("SEND_IN_THREADS", "SEND_MESSAGES_IN_THREADS", None),
    (
        "SEND_IN_POSTS",
        "SEND_MESSAGES_IN_THREADS",
        Some(PermContext::Forum),
    ),
    ("MANAGE_POSTS", "MANAGE_THREADS", Some(PermContext::Forum)),
    ("VIEW", "VIEW_CHANNEL", None),
    ("READ_HISTORY", "READ_MESSAGE_HISTORY", None),
    ("CREATE_INVITE", "CREATE_INSTANT_INVITE", None),
    ("MANAGE_SERVER", "MANAGE_GUILD", None),
    (
        "MANAGE_EMOJIS_AND_STICKERS",
        "MANAGE_GUILD_EXPRESSIONS",
        None,
    ),
    ("MANAGE_EXPRESSIONS", "MANAGE_GUILD_EXPRESSIONS", None),
    ("USE_SLASH_COMMANDS", "USE_APPLICATION_COMMANDS", None),
    ("USE_VOICE_ACTIVITY", "USE_VAD", None),
    ("USE_ACTIVITIES", "USE_EMBEDDED_ACTIVITIES", None),
    ("TIMEOUT_MEMBERS", "MODERATE_MEMBERS", None),
];

/// Names used when *writing* permissions for a forum (import, plan output), so
/// a forum reads the way the Discord client describes it.
const FORUM_DISPLAY: &[(&str, &str)] = &[
    ("SEND_MESSAGES", "CREATE_POSTS"),
    ("SEND_MESSAGES_IN_THREADS", "SEND_IN_THREADS"),
    ("MANAGE_THREADS", "MANAGE_POSTS"),
];

fn normalise_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| match c {
            ' ' | '-' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

fn bit_of(canonical: &str) -> Option<u32> {
    TABLE.iter().find(|(n, _)| *n == canonical).map(|(_, b)| *b)
}

/// Parse one permission name (canonical, alias, or `BIT_<n>`).
pub fn parse_perm(name: &str, ctx: PermContext) -> Result<Perms, String> {
    let norm = normalise_name(name);
    if let Some(bit) = bit_of(&norm) {
        return Ok(Perms(1 << bit));
    }
    if let Some((alias, canonical, only)) = ALIASES.iter().find(|(a, _, _)| *a == norm) {
        if let Some(only) = only {
            if *only != ctx {
                let hint = if *alias == "CREATE_POSTS" {
                    format!(
                        "`CREATE_POSTS` only applies to forum channels; use `{canonical}` \
                         (note: on a category it also affects text channels)"
                    )
                } else {
                    format!("`{alias}` only applies to forum channels; use `{canonical}`")
                };
                return Err(hint);
            }
        }
        let bit = bit_of(canonical).expect("alias targets are in TABLE");
        return Ok(Perms(1 << bit));
    }
    if let Some(n) = norm
        .strip_prefix("BIT_")
        .and_then(|n| n.parse::<u32>().ok())
    {
        if n < 64 {
            return Ok(Perms(1 << n));
        }
    }
    let candidates = TABLE
        .iter()
        .map(|(n, _)| *n)
        .chain(ALIASES.iter().map(|(a, _, _)| *a));
    let mut msg = format!("unknown permission `{name}`");
    if let Some(s) = crate::diag::suggest(&norm, candidates) {
        msg.push_str(&format!(" ({s})"));
    }
    Err(msg)
}

/// Render permission bits as names, in bit order.
pub fn names(perms: Perms, ctx: PermContext) -> Vec<String> {
    (0..64u32)
        .filter(|bit| perms.0 & (1 << bit) != 0)
        .map(|bit| {
            let canonical = TABLE.iter().find(|(_, b)| *b == bit).map(|(n, _)| *n);
            match canonical {
                Some(name) => {
                    if ctx == PermContext::Forum {
                        if let Some((_, friendly)) = FORUM_DISPLAY.iter().find(|(c, _)| *c == name)
                        {
                            return (*friendly).to_owned();
                        }
                    }
                    name.to_owned()
                }
                None => format!("BIT_{bit}"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_names_case_insensitively() {
        assert_eq!(
            parse_perm("view_channel", PermContext::Text),
            Ok(Perms::VIEW_CHANNEL)
        );
        assert_eq!(
            parse_perm("Manage Roles", PermContext::Role),
            Ok(Perms::MANAGE_ROLES)
        );
    }

    #[test]
    fn create_posts_maps_to_send_messages_on_forums_only() {
        assert_eq!(
            parse_perm("CREATE_POSTS", PermContext::Forum),
            Ok(Perms::SEND_MESSAGES)
        );
        let err = parse_perm("CREATE_POSTS", PermContext::Text).unwrap_err();
        assert!(err.contains("only applies to forum channels"), "{err}");
    }

    #[test]
    fn send_in_threads_maps_everywhere() {
        let expected = Perms(1 << 38);
        assert_eq!(
            parse_perm("SEND_IN_THREADS", PermContext::Text),
            Ok(expected)
        );
        assert_eq!(
            parse_perm("SEND_IN_THREADS", PermContext::Forum),
            Ok(expected)
        );
    }

    #[test]
    fn unknown_names_get_suggestions() {
        let err = parse_perm("VIEW_CHANEL", PermContext::Text).unwrap_err();
        assert!(err.contains("did you mean `VIEW_CHANNEL`"), "{err}");
    }

    #[test]
    fn unknown_bits_round_trip() {
        let p = Perms(1 << 60);
        let n = names(p, PermContext::Role);
        assert_eq!(n, vec!["BIT_60"]);
        assert_eq!(parse_perm(&n[0], PermContext::Role), Ok(p));
    }

    #[test]
    fn forum_names_use_friendly_aliases() {
        let p = Perms::SEND_MESSAGES.union(Perms::VIEW_CHANNEL);
        assert_eq!(
            names(p, PermContext::Forum),
            vec!["VIEW_CHANNEL", "CREATE_POSTS"]
        );
        assert_eq!(
            names(p, PermContext::Text),
            vec!["VIEW_CHANNEL", "SEND_MESSAGES"]
        );
    }

    #[test]
    fn table_has_no_duplicates() {
        for (i, (name, bit)) in TABLE.iter().enumerate() {
            for (other, other_bit) in &TABLE[i + 1..] {
                assert_ne!(name, other);
                assert_ne!(bit, other_bit);
            }
        }
    }
}
