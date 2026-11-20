//! What the game says to its players, in ICU MessageFormat: one catalog, `Messages`, a section
//! for each system that puts something into words, each message a `Phrase`. A sentence is always
//! one message, its arguments filled in by name, so its word order, plurals and casing belong to
//! the language it is written in rather than to the code that says it.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use formatjs_icu_messageformat::{IcuMessageFormat, Part, Value};
use formatjs_icu_messageformat_parser::types::{
    MessageFormatElement, NumberSkeleton, NumberSkeletonOrStyle,
};
use formatjs_icu_messageformat_parser::{ArgumentKind, message_arguments};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{
    Content, ContentFile, ContentFolder, Fixture, FolderReader, LoadStep, Loading, Nearby, RowCheck,
};
use crate::core::text::{LineText, SpanText};
use crate::systems::input::map::InputAction;

/// The catalog every game provides at `fixtures/game.messages.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum MessagesFixture {
    Game,
}

impl Fixture for MessagesFixture {
    type Row = Messages;
    const FOLDER: Option<&'static str> = None;
    const WHOLE_TABLE: bool = true;
}

/// Registers the catalog and makes the one a game loads, in its locale, what
/// `Content::built::<Messages>()` reads.
pub fn content(registry: &mut crate::core::content::ContentRegistry) {
    registry
        .fixture::<MessagesFixture>()
        .file::<Messages>()
        .hook(LoadStep::Files, localize);
}

/// The catalog of the game `content` is.
pub fn of(content: &Content) -> &Messages {
    content.built::<Messages>()
}

/// The catalog of the game `folder` holds, in its locale, read without the rest of the game: what
/// a client says while the game is still on its way.
pub fn read_alone(folder: Arc<dyn ContentFolder>) -> Option<Messages> {
    let reader = FolderReader::new(crate::systems::registry(), folder);
    let mut messages = reader
        .read::<Messages>(&MessagesFixture::Game.path())
        .ok()?;
    messages.sections_localize();
    Some(messages)
}

/// A locale, such as `en-US`: the language and region whose rules the catalog's plurals and
/// numbers follow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Locale(Arc<str>);

impl JsonSchema for Locale {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Locale".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "minLength": 2 })
    }
}

impl TryFrom<String> for Locale {
    type Error = String;

    fn try_from(tag: String) -> Result<Locale, String> {
        icu_locale_core::Locale::try_from_str(&tag)
            .map_err(|error| format!("{tag:?} is no locale ({error}): one such as \"en-US\""))?;
        Ok(Locale(tag.into()))
    }
}

impl Locale {
    /// What orders words as readers of the locale expect a list sorted by name to read.
    pub fn collator(&self) -> icu_collator::CollatorBorrowed<'static> {
        let locale = icu_locale_core::Locale::try_from_str(&self.0)
            .expect("a locale is read only from a tag that parses");
        icu_collator::Collator::try_new((&locale).into(), Default::default())
            .expect("the collation data built in covers every locale")
    }
}

impl From<Locale> for String {
    fn from(locale: Locale) -> String {
        locale.0.to_string()
    }
}

impl Default for Locale {
    fn default() -> Locale {
        Locale("und".into())
    }
}

/// One message the game says, in ICU MessageFormat: `Hello, {name}`,
/// `{count, plural, one {# stack} other {# stacks}}`. What says it fills its arguments through
/// the method its section declares for it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Phrase {
    source: Arc<str>,
    format: Arc<IcuMessageFormat>,
    /// What the message says when it takes no argument.
    plain: Option<Arc<str>>,
    locale: Locale,
}

impl Phrase {
    /// A message named `field` that says its name and shows each of its `arguments`.
    pub fn placeholder(
        field: &str,
        arguments: &[(&str, &[ArgumentKind])],
    ) -> Result<Phrase, String> {
        let named = field.replace('_', " ");
        let shown: Vec<String> = arguments
            .iter()
            .map(|(arg, uses)| {
                if uses.contains(&ArgumentKind::Value) {
                    format!("{{{arg}}}")
                } else {
                    format!("{{{arg}, number}}")
                }
            })
            .collect();
        Phrase::try_from(
            [named]
                .into_iter()
                .chain(shown)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// What a message that takes no argument says.
    pub fn text(&self) -> &str {
        self.plain.as_deref().unwrap_or(&self.source)
    }

    /// What the message says with `arguments` filled in, as plain words.
    pub fn string(&self, arguments: Vec<(&'static str, Plain)>) -> String {
        let values = arguments
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.value()))
            .collect::<HashMap<_, Value<String>>>();
        self.format
            .format_to_string(&*self.locale.0, &values)
            .unwrap_or_else(|error| self.unsaid(&error))
    }

    /// What the message says with `arguments` filled in, as a line that keeps what styles them.
    pub fn line(&self, arguments: Vec<(&'static str, Rich)>) -> LineText {
        let values = arguments
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.value()))
            .collect::<HashMap<_, Value<LineText>>>();
        match self.format.format_to_parts(&*self.locale.0, &values) {
            Ok(parts) => parts
                .into_iter()
                .fold(LineText::default(), |line, part| match part {
                    Part::Literal(text) => line.then(text),
                    Part::Object(more) => line.then(more),
                }),
            Err(error) => LineText::plain(self.unsaid(&error)),
        }
    }

    /// What the message says with `arguments` filled in, part by part, for a caller that draws
    /// each argument its own way.
    pub fn parts<T: Clone>(&self, arguments: Vec<(&'static str, T)>) -> Vec<MessagePart<T>> {
        let values = arguments
            .into_iter()
            .map(|(name, value)| (name.to_owned(), Value::Object(value)))
            .collect::<HashMap<_, Value<T>>>();
        match self.format.format_to_parts(&*self.locale.0, &values) {
            Ok(parts) => parts
                .into_iter()
                .map(|part| match part {
                    Part::Literal(words) => MessagePart::Words(words),
                    Part::Object(argument) => MessagePart::Argument(argument),
                })
                .collect(),
            Err(error) => vec![MessagePart::Words(self.unsaid(&error))],
        }
    }

    /// Checks that the message, at `pointer`, uses no argument but those `declared`, each only
    /// as its kind may be used.
    pub fn check(&self, pointer: &str, declared: &[(&str, &[ArgumentKind])], cx: &mut RowCheck) {
        for (name, uses) in message_arguments(self.format.get_ast()) {
            let Some((_, allowed)) = declared.iter().find(|(known, _)| *known == name) else {
                cx.error(
                    pointer,
                    "unknown argument",
                    format!("{{{name}}}"),
                    takes(declared),
                );
                continue;
            };
            for used in uses.iter().filter(|used| !allowed.contains(used)) {
                cx.error(
                    pointer,
                    "argument misused",
                    format!("{{{name}}} used as {}", kind_name(*used)),
                    format!(
                        "{{{name}}} used as {}",
                        allowed
                            .iter()
                            .map(|kind| kind_name(*kind))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ),
                );
            }
        }
        for (name, format) in number_formats(self.format.get_ast()) {
            if !shown_whole(format) {
                cx.error(
                    pointer,
                    "number format not shown",
                    format!("{{{name}, number, ::…}} with an option the game leaves out"),
                    "a number format of digits, grouping, rounding, percent, currency or unit: write a sign or any other wording in the message itself",
                );
            }
        }
    }

    pub fn localize(&mut self, locale: &Locale) {
        self.locale = locale.clone();
    }

    /// What is said where the message could not be formatted: its own source, so the failure is
    /// seen rather than silent.
    fn unsaid(&self, error: &formatjs_icu_messageformat::Error) -> String {
        bevy::log::error!("message {:?} could not be said: {error}", self.source);
        self.source.to_string()
    }
}

impl TryFrom<String> for Phrase {
    type Error = String;

    fn try_from(source: String) -> Result<Phrase, String> {
        if source.trim().is_empty() {
            return Err("blank: a message the game says has words".to_owned());
        }
        let format = IcuMessageFormat::try_new(&source)
            .map_err(|error| format!("not ICU MessageFormat: {}", error.message))?;
        let plain = message_arguments(format.get_ast())
            .is_empty()
            .then(|| format.format_to_string("und", &HashMap::new()).ok())
            .flatten()
            .map(Arc::from);
        Ok(Phrase {
            source: source.into(),
            format: Arc::new(format),
            plain,
            locale: Locale::default(),
        })
    }
}

impl From<Phrase> for String {
    fn from(phrase: Phrase) -> String {
        phrase.source.to_string()
    }
}

impl PartialEq for Phrase {
    fn eq(&self, other: &Phrase) -> bool {
        self.source == other.source && self.locale == other.locale
    }
}

impl fmt::Debug for Phrase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.source)
    }
}

impl JsonSchema for Phrase {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Phrase".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "minLength": 1 })
    }
}

/// A section of the catalog: what its messages may take, and the locale they are said in.
pub trait Section: Sized {
    fn check(&self, at: &str, cx: &mut RowCheck);
    fn localize(&mut self, locale: &Locale);
    /// Every message saying its own name and the arguments it takes: the placeholder a designer
    /// writes the game's words over.
    fn fresh() -> Result<Self, String>;
}

/// What an argument of a message holds, and so how the message may use it.
pub trait ArgumentKindOf {
    const USES: &'static [ArgumentKind];
}

/// A number, which a message may say, count in a plural or format as it likes.
pub struct Number(Plain);

/// Words, which a message may say or choose by in a select.
pub struct Text(String);

/// Which way a change goes: what a message shows a change's sign by, in its own words, as
/// `{direction, select, down {−} other {+}}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn of<T: PartialOrd + Default>(change: T) -> Direction {
        if change < T::default() {
            Direction::Down
        } else {
            Direction::Up
        }
    }
}

/// A line that keeps what styles it (the name of an item, a key), which a message may only say.
pub struct Line(LineText);

impl ArgumentKindOf for Number {
    const USES: &'static [ArgumentKind] = &[ArgumentKind::Number];
}

impl ArgumentKindOf for Text {
    const USES: &'static [ArgumentKind] = &[ArgumentKind::Value, ArgumentKind::Select];
}

impl ArgumentKindOf for Line {
    const USES: &'static [ArgumentKind] = &[ArgumentKind::Value];
}

/// An argument of a message said as plain words.
pub enum Plain {
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
}

/// A piece of a message said part by part: its own words, or one of the arguments it was given.
#[derive(Clone, Debug, PartialEq)]
pub enum MessagePart<T> {
    Words(String),
    Argument(T),
}

/// An argument of a message said as a line.
pub enum Rich {
    Plain(Plain),
    Line(LineText),
}

impl Plain {
    fn value<T>(self) -> Value<T> {
        match self {
            Plain::Integer(number) => Value::Integer(number),
            Plain::Unsigned(number) => Value::Unsigned(number),
            Plain::Float(number) => Value::Number(number),
            Plain::Text(text) => Value::String(text),
        }
    }
}

impl Rich {
    fn value(self) -> Value<LineText> {
        match self {
            Rich::Plain(plain) => plain.value(),
            Rich::Line(line) => Value::Object(line),
        }
    }
}

impl From<Number> for Plain {
    fn from(number: Number) -> Plain {
        number.0
    }
}

impl From<Text> for Plain {
    fn from(text: Text) -> Plain {
        Plain::Text(text.0)
    }
}

impl From<Number> for Rich {
    fn from(number: Number) -> Rich {
        Rich::Plain(number.0)
    }
}

impl From<Text> for Rich {
    fn from(text: Text) -> Rich {
        Rich::Plain(Plain::Text(text.0))
    }
}

impl From<Line> for Rich {
    fn from(line: Line) -> Rich {
        Rich::Line(line.0)
    }
}

macro_rules! numbers {
    ($($kind:ident: $($number:ty),+;)+) => {
        $($(
            impl From<$number> for Number {
                fn from(number: $number) -> Number {
                    Number(Plain::$kind(number.into()))
                }
            }
        )+)+
    };
}

numbers! {
    Integer: i8, i16, i32, i64;
    Unsigned: u8, u16, u32, u64;
    Float: f32, f64;
}

impl From<usize> for Number {
    fn from(number: usize) -> Number {
        Number(Plain::Unsigned(number as u64))
    }
}

impl From<Direction> for Text {
    fn from(direction: Direction) -> Text {
        Text(
            match direction {
                Direction::Up => "up",
                Direction::Down => "down",
            }
            .to_owned(),
        )
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Text {
        Text(text.to_owned())
    }
}

impl From<String> for Text {
    fn from(text: String) -> Text {
        Text(text)
    }
}

impl From<&String> for Text {
    fn from(text: &String) -> Text {
        Text(text.clone())
    }
}

impl From<LineText> for Line {
    fn from(line: LineText) -> Line {
        Line(line)
    }
}

impl From<SpanText> for Line {
    fn from(span: SpanText) -> Line {
        Line(span.into())
    }
}

impl From<InputAction> for Line {
    fn from(action: InputAction) -> Line {
        Line(SpanText::from(action).into())
    }
}

impl From<&str> for Line {
    fn from(text: &str) -> Line {
        Line(text.into())
    }
}

impl From<String> for Line {
    fn from(text: String) -> Line {
        Line(text.into())
    }
}

/// Declares a section of the catalog: a struct of messages, each with the arguments it takes.
/// A message listed bare takes none and is read as `&str`; `name(arg: Kind, …)` takes plain
/// arguments and is said as a `String`; `name[arg: Kind, …]` may take lines and is said as a
/// `LineText`. A kind is `Number`, `Text` or `Line`. Each message's doc is the one designers read.
macro_rules! messages {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident
                $(($($arg:ident: $kind:ident),+ $(,)?))?
                $([$($rich:ident: $rich_kind:ident),+ $(,)?])?
                $({$($part:ident),+ $(,)?})?
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(schemars::JsonSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(
                $(#[$field_meta])*
                $field: $crate::systems::messages::Phrase,
            )*
        }

        impl $name {
            $(
                $crate::systems::messages::phrase_method!(
                    $(#[$field_meta])* $field
                    $(($($arg: $kind),+))?
                    $([$($rich: $rich_kind),+])?
                    $({$($part),+})?
                );
            )*
        }

        impl $crate::systems::messages::Section for $name {
            fn check(&self, at: &str, cx: &mut $crate::core::content::RowCheck) {
                $(
                    self.$field.check(
                        &format!("{at}/{}", stringify!($field)),
                        &[
                            $($((
                                stringify!($arg),
                                <$crate::systems::messages::$kind as
                                    $crate::systems::messages::ArgumentKindOf>::USES,
                            )),+)?
                            $($((
                                stringify!($rich),
                                <$crate::systems::messages::$rich_kind as
                                    $crate::systems::messages::ArgumentKindOf>::USES,
                            )),+)?
                            $($((
                                stringify!($part),
                                <$crate::systems::messages::Line as
                                    $crate::systems::messages::ArgumentKindOf>::USES,
                            )),+)?
                        ],
                        cx,
                    );
                )*
            }

            fn localize(&mut self, locale: &$crate::systems::messages::Locale) {
                $(self.$field.localize(locale);)*
            }

            fn fresh() -> Result<$name, String> {
                Ok($name {
                    $(
                        $field: $crate::systems::messages::Phrase::placeholder(
                            stringify!($field),
                            &[
                                $($((
                                    stringify!($arg),
                                    <$crate::systems::messages::$kind as
                                        $crate::systems::messages::ArgumentKindOf>::USES,
                                )),+)?
                                $($((
                                    stringify!($rich),
                                    <$crate::systems::messages::$rich_kind as
                                        $crate::systems::messages::ArgumentKindOf>::USES,
                                )),+)?
                                $($((
                                    stringify!($part),
                                    <$crate::systems::messages::Line as
                                        $crate::systems::messages::ArgumentKindOf>::USES,
                                )),+)?
                            ],
                        )?,
                    )*
                })
            }
        }
    };
}

macro_rules! phrase_method {
    ($(#[$meta:meta])* $field:ident) => {
        $(#[$meta])*
        pub fn $field(&self) -> &str {
            self.$field.text()
        }
    };
    ($(#[$meta:meta])* $field:ident ($($arg:ident: $kind:ident),+)) => {
        $(#[$meta])*
        pub fn $field(&self, $($arg: impl Into<$crate::systems::messages::$kind>),+) -> String {
            self.$field.string(vec![$((
                stringify!($arg),
                $crate::systems::messages::Plain::from($arg.into()),
            )),+])
        }
    };
    ($(#[$meta:meta])* $field:ident [$($arg:ident: $kind:ident),+]) => {
        $(#[$meta])*
        pub fn $field(
            &self,
            $($arg: impl Into<$crate::systems::messages::$kind>),+
        ) -> $crate::core::text::LineText {
            self.$field.line(vec![$((
                stringify!($arg),
                $crate::systems::messages::Rich::from($arg.into()),
            )),+])
        }
    };
    ($(#[$meta:meta])* $field:ident {$($arg:ident),+}) => {
        $(#[$meta])*
        pub fn $field<T: Clone>(
            &self,
            $($arg: T),+
        ) -> Vec<$crate::systems::messages::MessagePart<T>> {
            self.$field.parts(vec![$((stringify!($arg), $arg)),+])
        }
    };
}

pub(crate) use {messages, phrase_method};

macro_rules! catalog {
    ($($section:ident: $ty:ty),+ $(,)?) => {
        /// What the game says to its players, in the locale its plurals and numbers follow: a
        /// section for each system that puts something into words.
        #[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Messages {
            pub locale: Locale,
            $(pub $section: $ty,)+
        }

        impl Messages {
            fn sections_check(&self, cx: &mut RowCheck) {
                $(Section::check(&self.$section, concat!("/", stringify!($section)), cx);)+
            }

            fn sections_localize(&mut self) {
                let locale = self.locale.clone();
                $(Section::localize(&mut self.$section, &locale);)+
            }

            fn sections_fresh() -> Result<Messages, String> {
                Ok(Messages {
                    locale: Locale::default(),
                    $($section: Section::fresh()?,)+
                })
            }
        }
    };
}

use crate::systems::*;

catalog! {
    interface: interface::InterfaceMessages,
    time: duration::TimeMessages,
    stat: stat::StatMessages,
    quest: quest::QuestMessages,
    dialogue: dialogue::DialogueMessages,
    item: item::ItemMessages,
    resource: resource::ResourceMessages,
    prompt: prompt::PromptMessages,
    crafting: crafting::CraftingMessages,
    command: command::CommandMessages,
    save: save::SaveMessages,
    requests: social_request::RequestMessages,
    friends: friend::FriendMessages,
    mail: mail::MailMessages,
    party: party::PartyMessages,
    chat: chat::ChatMessages,
    mount: mount::MountMessages,
    skill: skill::SkillMessages,
    instance: area_instance::InstanceMessages,
    deliver: item::deliver::DeliverMessages,
    trade: trade::TradeMessages,
    market: market::MarketMessages,
    status: status::StatusMessages,
    condition: requirement::ConditionMessages,
    class: class::ClassMessages,
    shop: shop::ShopMessages,
    equipment: equipment::EquipmentMessages,
    belongings: belongings::BelongingsMessages,
    screen: screen::ScreenMessages,
    settings: settings::SettingsMessages,
    input: input::InputMessages,
    hud: hud::HudMessages,
    terminal: terminal::TerminalMessages,
    history: history::HistoryMessages,
    notification: notification::NotificationMessages,
    map: area_map::MapMessages,
    emote: emote::EmoteMessages,
    actor: actor::ActorMessages,
    player: player::PlayerMessages,
}

impl ContentFile for Messages {
    const TABLE: &'static str = "messages";

    fn fresh(_: &Nearby) -> Result<Messages, String> {
        Messages::sections_fresh()
    }

    fn check(&self, cx: &mut RowCheck) {
        self.sections_check(cx);
    }
}

fn localize(loading: &mut Loading) {
    let content = loading.content();
    let path = MessagesFixture::Game.path();
    let Some(messages) = content
        .table::<Messages>()
        .id_of(&path)
        .and_then(|id| content.table::<Messages>().get(id))
    else {
        return;
    };
    let mut messages = messages.clone();
    messages.sections_localize();
    loading.built(messages);
}

fn takes(declared: &[(&str, &[ArgumentKind])]) -> String {
    if declared.is_empty() {
        return "no argument: this message takes none".to_owned();
    }
    let names: Vec<String> = declared
        .iter()
        .map(|(name, _)| format!("{{{name}}}"))
        .collect();
    format!("one of {}", names.join(", "))
}

/// Every number a message formats by a skeleton, by the name of its argument.
fn number_formats(ast: &[MessageFormatElement]) -> Vec<(&str, &NumberSkeleton)> {
    ast.iter()
        .flat_map(|element| match element {
            MessageFormatElement::Number(number) => match &number.style {
                Some(NumberSkeletonOrStyle::Skeleton(skeleton)) => {
                    vec![(number.value.as_str(), skeleton)]
                }
                _ => Vec::new(),
            },
            MessageFormatElement::Select(select) => select
                .options
                .values()
                .flat_map(|option| number_formats(&option.value))
                .collect(),
            MessageFormatElement::Plural(plural) => plural
                .options
                .values()
                .flat_map(|option| number_formats(&option.value))
                .collect(),
            MessageFormatElement::Tag(tag) => number_formats(&tag.children),
            _ => Vec::new(),
        })
        .collect()
}

/// Whether a player sees all `skeleton` asks for: the formatter says a number with its digits,
/// grouping, rounding and style, and leaves out every other option a skeleton may set.
fn shown_whole(skeleton: &NumberSkeleton) -> bool {
    let mut unshown = skeleton.parsed_options.clone();
    let base = &mut unshown.base;
    base.style = None;
    base.currency = None;
    base.unit = None;
    base.use_grouping = None;
    base.minimum_integer_digits = None;
    base.minimum_fraction_digits = None;
    base.maximum_fraction_digits = None;
    base.trailing_zero_display = None;
    base.rounding_mode = None;
    unshown == Default::default()
}

fn kind_name(kind: ArgumentKind) -> &'static str {
    match kind {
        ArgumentKind::Value => "a value",
        ArgumentKind::Number => "a number or plural",
        ArgumentKind::DateTime => "a date or time",
        ArgumentKind::Select => "a select",
        ArgumentKind::Tag => "a tag",
    }
}
