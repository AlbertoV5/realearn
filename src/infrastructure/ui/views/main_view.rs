use crate::domain::Session;
use crate::infrastructure::common::bindings::root::{ID_MAIN_DIALOG, ID_MAPPINGS_DIALOG};
use crate::infrastructure::ui::framework::{
    convert_dialog_units_to_pixels, open_view, ViewListener, Window,
};
use crate::infrastructure::ui::views::constants::OPTIMAL_MAIN_WINDOW_WIDTH_AND_HEIGHT;
use crate::infrastructure::ui::views::HeaderView;
use c_str_macro::c_str;
use reaper_high::Reaper;
use reaper_low::{raw, Swell};
use std::cell::{Cell, RefCell};
use std::ptr::null_mut;
use std::rc::Rc;

/// The complete ReaLearn view containing everything.
#[derive(Debug)]
pub struct MainView {
    /// The upper view contaning lots of general buttons.
    header_view: Rc<HeaderView>,
    /// This is for HiDPI handling on Windows.
    ///
    /// We will set this to true whenever the plug-in window opens and shortly after that set it to
    /// false to indicate that the size calculation has been done (small state machine).
    determining_best_dimensions: Cell<bool>,
    /// `Plugin#get_editor()` must return a Box of something 'static, so it's impossible to take a
    /// reference here. Why? Because a reference needs a lifetime. Any non-static lifetime would
    /// not satisfy the 'static requirement. Why not require a 'static reference then? Simply
    /// because we don't have a session object with static lifetime. The session object is
    /// owned by the `Plugin` object, which itself doesn't have a static lifetime. The only way
    /// to get a 'static session would be to not let the plugin object own the session but to
    /// define a static global. This, however, would be a far worse design than just using a
    /// smart pointer here. So using a smart pointer is the best we can do really.
    ///
    /// This is not the only reason why taking a reference here is not feasible. During the
    /// lifecycle of a ReaLearn session we need mutable access to the session both from the
    /// editor (of course) and from the plugin (e.g. when REAPER wants us to load some data).
    /// When using references, Rust's borrow checker wouldn't let that happen. We can't do anything
    /// about this multiple-access requirement, it's just how the VST plugin API works (and
    /// many other similar plugin interfaces as well - for good reasons). And being a plugin we
    /// have to conform.
    ///
    /// Fortunately, we know that actually both DAW-plugin interaction (such as loading data) and
    /// UI interaction happens in the main thread, in the so called main loop. So there's no
    /// need for using a thread-safe smart pointer here. We even can and also should satisfy
    /// the borrow checker, meaning that if the session is mutably accessed at a given point in
    /// time, it is not accessed from another point as well. This can happen even in a
    /// single-threaded environment because functions can call other functions and thereby
    /// accessing the same data - just in different stack positions. Just think of reentrancy.
    /// Fortunately this is something we can control. And we should, because when this kind of
    /// parallel access happens, this can lead to strange bugs which are particularly hard to
    /// find.
    ///
    /// Unfortunately we can't make use of Rust's compile time borrow checker because there's no
    /// way that the compiler understands what's going on here. Why? For one thing, because of
    /// the VST plugin API design. But first and foremost because we use the FFI, which means
    /// we interface with non-Rust code, so Rust couldn't get the complete picture even if the
    /// plugin system would be designed in a different way. However, we *can* use Rust's
    /// runtime borrow checker `RefCell`. And we should, because it gives us fail-fast
    /// behavior. It will let us know immediately when we violated that safety rule.
    /// TODO-low We must take care, however, that REAPER will not crash as a result, that would be
    /// very  bad.  See https://github.com/RustAudio/vst-rs/issues/122
    session: Rc<RefCell<Session<'static>>>,
}

impl MainView {
    pub fn new(session: Rc<RefCell<Session<'static>>>) -> MainView {
        MainView {
            header_view: Rc::new(HeaderView::new(session.clone())),
            determining_best_dimensions: false.into(),
            session,
        }
    }

    pub fn open(self: Rc<Self>, parent_window: Window) {
        open_view(self, ID_MAIN_DIALOG, parent_window);
    }

    pub fn resize_and_open(self: Rc<Self>, parent_window: Window) {
        // On Windows, the first time opening the dialog window is just to determine the best
        // dimensions based on HiDPI settings.
        #[cfg(target_family = "windows")]
        self.determining_best_dimensions.replace(true);
        self.open(parent_window)
    }
}

impl ViewListener for MainView {
    fn opened(self: Rc<Self>, window: Window) {
        if self.determining_best_dimensions.replace(false) {
            // The dialog has been opened by user request. First calculate optimal size.
            let (width, height) =
                convert_dialog_units_to_pixels(window, OPTIMAL_MAIN_WINDOW_WIDTH_AND_HEIGHT);
            // Close this probe window and initiate a plugin resize => When resized, the plugin
            // calls show().
            window.close();
            // TODO Then we make the PluginEditor return a new size the next time it's opened.
            // TODO Then we open it again: Challenge: We need to obtain the plug-in window (as
            //  parent window).
            // self.open()
            return;
        }
        // Optimal size has been calculated and window has been reopened. Now add sub views!
        open_view(self.header_view.clone(), ID_MAPPINGS_DIALOG, window);
    }
}
