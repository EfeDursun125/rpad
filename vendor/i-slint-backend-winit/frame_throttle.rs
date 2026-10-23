// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::rc::Weak;

use crate::winitwindowadapter::WinitWindowAdapter;

pub fn create_frame_throttle(
    _window_adapter: Weak<WinitWindowAdapter>,
    _winit_window: &winit::window::Window,
    _is_wayland: bool,
) -> Box<dyn FrameThrottle> {
    Box::new(EventLoopFrameThrottle)
}

pub trait FrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window);
}

struct EventLoopFrameThrottle;

impl FrameThrottle for EventLoopFrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window) {
        // The adapter gates rendering and the event loop waits for its precise deadline.
        winit_window.request_redraw();
    }
}
