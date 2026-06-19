//! The component library: one module per conceptual component, each a controlled view builder over
//! [`bevy_view`](bevy_view). Shared mechanism (recipes, theming, controlled-value plumbing) lives in
//! [`crate::utils`]; the per-theme token values live in [`crate::themes`].

mod accordion;
mod alert_dialog;
mod avatar;
mod button;
mod card;
mod checkbox;
mod collapsible;
mod dialog;
mod popover;
mod progress;
mod radio_group;
mod scroll_area;
mod separator;
mod slider;
mod sonner;
mod switch;
mod tabs;
mod text;
mod tooltip;

pub use accordion::{
    Accordion, AccordionContent, AccordionHeader, AccordionItem, AccordionTrigger,
};
pub use alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
    AlertDialogOutlet, AlertDialogOverlay, AlertDialogTitle, AlertDialogTrigger,
};
pub use avatar::{Avatar, AvatarFallback, AvatarImage};
pub use button::Button;
pub use card::Card;
pub use checkbox::{Check, Checkbox, CheckboxIndicator};
pub use collapsible::{Collapsible, CollapsibleContent, CollapsibleTrigger};
pub use dialog::{
    Dialog, DialogClose, DialogContent, DialogDescription, DialogOutlet, DialogOverlay,
    DialogTitle, DialogTrigger,
};
pub use popover::{Popover, PopoverClose, PopoverContent, PopoverOutlet, PopoverTrigger};
pub use progress::{Progress, ProgressIndicator};
pub use radio_group::{RadioGroup, RadioGroupIndicator, RadioGroupItem};
pub(crate) use scroll_area::sync_scrollbars;
pub use scroll_area::{
    ScrollArea, ScrollAreaCorner, ScrollAreaScrollbar, ScrollAreaThumb, ScrollAreaViewport,
};
pub use separator::Separator;
pub use slider::{Slider, SliderRange, SliderThumb, SliderTrack};
pub use sonner::{SonnerClose, SonnerPosition, Toast, Toaster};
pub use switch::{Switch, SwitchThumb};
pub use tabs::{Tabs, TabsContent, TabsList, TabsTrigger};
pub use text::Text;
pub use tooltip::{
    Tooltip, TooltipConfig, TooltipContent, TooltipOutlet, TooltipProvider, TooltipTrigger,
    open_due_tooltips,
};
