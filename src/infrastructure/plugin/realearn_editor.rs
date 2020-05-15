use crate::domain::Session;
use crate::infrastructure::ui::views::MainView;
use std::cell::RefCell;

use crate::infrastructure::ui::framework::{Pixels, Window};
use reaper_low::raw::HWND;
use std::os::raw::c_void;
use std::rc::Rc;
use vst::editor::Editor;

pub struct RealearnEditor {
    open: bool,
    main_view: Rc<MainView>,
    width_and_height: (Pixels, Pixels),
}

impl RealearnEditor {
    pub fn new(session: Rc<RefCell<Session<'static>>>) -> RealearnEditor {
        RealearnEditor {
            open: false,
            main_view: Rc::new(MainView::new(session)),
            width_and_height: (Pixels(1200), Pixels(600)),
        }
    }
}

impl Editor for RealearnEditor {
    fn size(&self) -> (i32, i32) {
        (1200, 600)
    }

    fn position(&self) -> (i32, i32) {
        (0, 0)
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn open(&mut self, parent: *mut c_void) -> bool {
        self.main_view
            .clone()
            .resize_and_open(Window::new(parent as HWND));
        self.open = true;
        true
    }

    fn is_open(&mut self) -> bool {
        self.open
    }
}
