// SPDX-License-Identifier: AGPL-3.0-or-later

//! The menu bar, as AppKit's own: an `NSMenu` at the top of the screen, built from the menus
//! `App` makes out of the same model the egui bar draws on Linux.
//!
//! **What changes on the way in** is where a Mac keeps things. Preferences and Quit leave the
//! app's own menus for the application menu, beside About, Services and Hide, under the
//! names and keys every Mac app gives them — Settings… on `⌘,` and Quit on `⌘Q` — and a
//! Window menu follows the app's own. Help's About and Licences are answered by AppKit's
//! standard About panel, which shows `Credits.rtf` from the bundle, so the Mac's Help menu
//! holds the rest of Help: Keyboard shortcuts… and Report a problem….
//! Everything else is the model, entry for entry: the enables, the ticks, the shortcuts and
//! the action each one sends.
//!
//! **An entry is chosen on the main thread, outside any frame**, by AppKit calling the one
//! Objective-C class here. It only writes down which entry that was and wakes the editor for
//! a frame; the frame takes the list and answers it the way it answers the egui bar.
//!
//! **A key equivalent is AppKit's before it is the window's.** `⌘Z` on Undo reaches the menu
//! and never reaches winit, which is what makes the entry and its key one path. It is also
//! why the clipboard three need care, which `App` takes: see `app::frame`.

use crate::platform::menu::{Entry, Item, Key, Menu, Role};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
    NSMenuItem, NSPasteboard, NSPasteboardTypeString,
};
use objc2_foundation::{NSObject, NSString};
use std::cell::RefCell;

/// What the target writes into: the tags chosen since the last frame took them, and what
/// wakes the editor so that frame happens.
struct Picks {
    chosen: RefCell<Vec<isize>>,
    wake: Box<dyn Fn()>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and this adds no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SupersilviaMenuTarget"]
    #[ivars = Picks]
    /// Every entry's target: writes down which one was chosen.
    struct Target;

    impl Target {
        // SAFETY: `-(void)pick:(NSMenuItem *)sender`, the signature AppKit sends a menu item's
        // action with.
        #[unsafe(method(pick:))]
        fn pick(&self, sender: &NSMenuItem) {
            self.ivars().chosen.borrow_mut().push(sender.tag());
            (self.ivars().wake)();
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, wake: Box<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Picks {
            chosen: RefCell::new(Vec::new()),
            wake,
        });
        // SAFETY: `init` on a freshly allocated `NSObject` subclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// The app's menu bar, while it is AppKit's.
pub struct Bar {
    mtm: MainThreadMarker,
    target: Retained<Target>,
    /// The menus the bar was last built from. A frame whose menus are the same builds nothing.
    shown: Vec<Menu>,
}

impl Bar {
    /// Take over the menu bar. `None` off the main thread, where AppKit's menus cannot be
    /// touched — a test harness's thread, which keeps the egui bar.
    pub fn install(wake: impl Fn() + 'static) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        Some(Self {
            mtm,
            target: Target::new(mtm, Box::new(wake)),
            shown: Vec::new(),
        })
    }

    /// Show these menus, rebuilding the bar only when they differ from the last ones.
    pub fn show(&mut self, menus: &[Menu]) {
        if self.shown == menus {
            return;
        }
        self.build(menus);
        self.shown = menus.to_vec();
    }

    /// The tag of every entry chosen from the bar since the last call, oldest first.
    pub fn take(&self) -> Vec<usize> {
        self.target
            .ivars()
            .chosen
            .take()
            .into_iter()
            .filter_map(|tag| usize::try_from(tag).ok())
            .collect()
    }

    /// The text on the system clipboard, or nothing. What `⌘V` would have pasted into a text
    /// field, had the Paste entry not taken the key first.
    pub fn pasteboard(&self) -> String {
        // SAFETY: an AppKit constant, initialized before any code of ours runs.
        let kind = unsafe { NSPasteboardTypeString };
        NSPasteboard::generalPasteboard()
            .stringForType(kind)
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    fn build(&self, menus: &[Menu]) {
        let mtm = self.mtm;
        let bar = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(""));

        // The model's own menus, less the two entries a Mac keeps in the application menu.
        let (settings, quit, menus) = lift(menus);

        let services = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Services"));
        let app = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("supersilvia"));
        app.addItem(&standard(
            mtm,
            "About supersilvia",
            sel!(orderFrontStandardAboutPanel:),
            "",
        ));
        app.addItem(&NSMenuItem::separatorItem(mtm));
        if let Some(settings) = settings {
            let item = self.entry(&Entry {
                label: "Settings…".to_owned(),
                ..settings
            });
            item.setKeyEquivalent(&NSString::from_str(","));
            item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
            app.addItem(&item);
            app.addItem(&NSMenuItem::separatorItem(mtm));
        }
        let item = submenu_item(mtm, "Services", &services);
        app.addItem(&item);
        app.addItem(&NSMenuItem::separatorItem(mtm));
        app.addItem(&standard(mtm, "Hide supersilvia", sel!(hide:), "h"));
        let others = standard(mtm, "Hide Others", sel!(hideOtherApplications:), "h");
        others.setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command | NSEventModifierFlags::Option,
        );
        app.addItem(&others);
        app.addItem(&standard(mtm, "Show All", sel!(unhideAllApplications:), ""));
        if let Some(quit) = quit {
            app.addItem(&NSMenuItem::separatorItem(mtm));
            let item = self.entry(&Entry {
                label: "Quit supersilvia".to_owned(),
                ..quit
            });
            item.setKeyEquivalent(&NSString::from_str("q"));
            item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
            app.addItem(&item);
        }
        bar.addItem(&submenu_item(mtm, "supersilvia", &app));

        for menu in &menus {
            let sub = self.menu(menu);
            let item = submenu_item(mtm, &menu.title, &sub);
            if let Some(hint) = &menu.hint {
                item.setToolTip(Some(&NSString::from_str(hint)));
            }
            bar.addItem(&item);
        }

        // The Window menu every Mac app has. AppKit lists the open windows in it itself.
        let window = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Window"));
        window.addItem(&standard(mtm, "Minimize", sel!(performMiniaturize:), "m"));
        window.addItem(&standard(mtm, "Zoom", sel!(performZoom:), ""));
        window.addItem(&NSMenuItem::separatorItem(mtm));
        window.addItem(&standard(
            mtm,
            "Bring All to Front",
            sel!(arrangeInFront:),
            "",
        ));
        bar.addItem(&submenu_item(mtm, "Window", &window));

        let application = NSApplication::sharedApplication(mtm);
        application.setMainMenu(Some(&bar));
        application.setServicesMenu(Some(&services));
        application.setWindowsMenu(Some(&window));
    }

    /// One of the model's menus, or submenus. Its entries are enabled by the model and by
    /// nothing else, so AppKit's own validation is off.
    fn menu(&self, model: &Menu) -> Retained<NSMenu> {
        let menu = NSMenu::initWithTitle(self.mtm.alloc(), &NSString::from_str(&model.title));
        menu.setAutoenablesItems(false);
        for item in tidy(&model.items) {
            match item {
                Item::Separator => menu.addItem(&NSMenuItem::separatorItem(self.mtm)),
                Item::Submenu(sub) => {
                    menu.addItem(&submenu_item(self.mtm, &sub.title, &self.menu(sub)));
                }
                Item::Entry(entry) => menu.addItem(&self.entry(entry)),
            }
        }
        menu
    }

    /// One entry, telling the target its tag when it is chosen.
    fn entry(&self, entry: &Entry) -> Retained<NSMenuItem> {
        let (key, modifiers) = entry.key.as_ref().map_or_else(
            || (String::new(), NSEventModifierFlags::empty()),
            key_equivalent,
        );
        let action = entry.tag.map(|_| sel!(pick:));
        // SAFETY: `pick:` is a method `Target` defines, and the item is given that target.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                self.mtm.alloc(),
                &NSString::from_str(&entry.label),
                action,
                &NSString::from_str(&key),
            )
        };
        item.setKeyEquivalentModifierMask(modifiers);
        if let Some(tag) = entry.tag.and_then(|tag| isize::try_from(tag).ok()) {
            let target: &AnyObject = &self.target;
            // SAFETY: `target` answers `pick:`, and `Bar` keeps it alive for as long as the
            // bar can be clicked. A menu item's target is weak, so were it ever gone, the
            // action would go up the responder chain and find nothing to answer it.
            unsafe { item.setTarget(Some(target)) };
            item.setTag(tag);
        }
        item.setEnabled(entry.enabled && entry.tag.is_some());
        if let Some(on) = entry.ticked {
            item.setState(if on {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        if let Some(hint) = &entry.hint {
            item.setToolTip(Some(&NSString::from_str(hint)));
        }
        item
    }
}

/// An item AppKit answers itself, by sending `action` up the responder chain to the window or
/// the application.
fn standard(mtm: MainThreadMarker, title: &str, action: Sel, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: every selector passed here is one `NSApplication` or `NSWindow` answers, and the
    // item has no target, so the responder chain finds whichever does.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        )
    }
}

/// An item that opens `menu`.
fn submenu_item(mtm: MainThreadMarker, title: &str, menu: &NSMenu) -> Retained<NSMenuItem> {
    // SAFETY: no action, so nothing is sent anywhere.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            None,
            &NSString::from_str(""),
        )
    };
    item.setSubmenu(Some(menu));
    item
}

/// A key as AppKit's key equivalent: the character, and the modifiers held with it.
fn key_equivalent(key: &Key) -> (String, NSEventModifierFlags) {
    let mut flags = NSEventModifierFlags::empty();
    for (held, flag) in [
        (key.command, NSEventModifierFlags::Command),
        (key.shift, NSEventModifierFlags::Shift),
        (key.alt, NSEventModifierFlags::Option),
        (key.ctrl, NSEventModifierFlags::Control),
    ] {
        if held {
            flags |= flag;
        }
    }
    (key.character.clone(), flags)
}

/// The menus with Settings and Quit taken out, and those two entries.
fn lift(menus: &[Menu]) -> (Option<Entry>, Option<Entry>, Vec<Menu>) {
    let mut settings = None;
    let mut quit = None;
    let menus = menus
        .iter()
        .map(|menu| Menu {
            items: menu
                .items
                .iter()
                .filter(|item| match item {
                    Item::Entry(entry) if entry.role == Role::Settings => {
                        settings = Some(entry.clone());
                        false
                    }
                    Item::Entry(entry) if entry.role == Role::Quit => {
                        quit = Some(entry.clone());
                        false
                    }
                    // The standard About panel answers both: its `Credits.rtf` is the
                    // licences in brief, and names the folder in the bundle holding them all.
                    Item::Entry(entry) if matches!(entry.role, Role::About | Role::Licences) => {
                        false
                    }
                    _ => true,
                })
                .cloned()
                .collect(),
            ..menu.clone()
        })
        // A menu left with nothing in it goes with what it held.
        .filter(|menu| !tidy(&menu.items).is_empty())
        .collect();
    (settings, quit, menus)
}

/// The items, less any separator that no longer separates anything: at either end, or beside
/// another. Taking Quit out of Project leaves one hanging.
fn tidy(items: &[Item]) -> Vec<&Item> {
    let mut kept: Vec<&Item> = Vec::new();
    for item in items {
        let rule = matches!(item, Item::Separator);
        if rule
            && kept
                .last()
                .is_none_or(|last| matches!(last, Item::Separator))
        {
            continue;
        }
        kept.push(item);
    }
    if kept
        .last()
        .is_some_and(|last| matches!(last, Item::Separator))
    {
        kept.pop();
    }
    kept
}
