use std::time::{Duration, Instant};

use nivora_platform::{InputEvent, Key, Point};
use vitasdk_sys::*;

use crate::ui::Action;

const DIRECTIONS: u32 = SCE_CTRL_UP | SCE_CTRL_DOWN | SCE_CTRL_LEFT | SCE_CTRL_RIGHT;
const KEYS: &[(u32, Key)] = &[
    (SCE_CTRL_UP, Key::Up),
    (SCE_CTRL_DOWN, Key::Down),
    (SCE_CTRL_LEFT, Key::Left),
    (SCE_CTRL_RIGHT, Key::Right),
    (SCE_CTRL_CIRCLE, Key::Accept),
    (SCE_CTRL_CROSS, Key::Back),
];

pub(super) enum VitaInputEvent {
    Ui(InputEvent),
    Shortcut(Action),
    CancelPointer,
}

pub(super) struct VitaInput {
    previous: u32,
    repeat: Repeat,
    panel: TouchPanel,
    touch: Option<(u8, Point)>,
    touch_ready: bool,
}

impl VitaInput {
    pub fn new() -> Self {
        unsafe {
            sceCtrlSetSamplingMode(SCE_CTRL_MODE_ANALOG);
            sceTouchSetSamplingState(SCE_TOUCH_PORT_FRONT, SCE_TOUCH_SAMPLING_STATE_START);
        }
        Self {
            previous: buttons(),
            repeat: Repeat::default(),
            panel: TouchPanel::front(),
            touch: None,
            touch_ready: false,
        }
    }

    pub fn reset(&mut self) {
        self.previous = buttons();
        self.repeat = Repeat::default();
        self.touch = None;
        self.touch_ready = false;
    }

    pub fn poll(&mut self, now: Instant) -> Vec<VitaInputEvent> {
        let mut events = Vec::with_capacity(8);
        let current = buttons();
        let pressed = current & !self.previous;
        let released = self.previous & !current;
        let repeated = self.repeat.poll(current & DIRECTIONS, now);
        for &(mask, key) in KEYS {
            if (pressed | repeated) & mask != 0 {
                events.push(VitaInputEvent::Ui(InputEvent::KeyDown(key)));
            }
            if released & mask != 0 {
                events.push(VitaInputEvent::Ui(InputEvent::KeyUp(key)));
            }
        }
        for (mask, action) in [
            (SCE_CTRL_TRIANGLE, Action::DeleteSelected),
            (SCE_CTRL_SELECT, Action::Settings),
            (SCE_CTRL_LTRIGGER, Action::PageUp),
            (SCE_CTRL_RTRIGGER, Action::PageDown),
            (SCE_CTRL_START, Action::Refresh),
        ] {
            if pressed & mask != 0 {
                events.push(VitaInputEvent::Shortcut(action));
            }
        }
        self.previous = current;

        let mut data = unsafe { std::mem::zeroed::<SceTouchData>() };
        if unsafe { sceTouchPeek(SCE_TOUCH_PORT_FRONT, &mut data, 1) } < 1 {
            if self.touch.take().is_some() {
                events.push(VitaInputEvent::CancelPointer);
            }
            self.touch_ready = false;
            return events;
        }
        let reports = &data.report[..(data.reportNum as usize).min(data.report.len())];
        if reports.is_empty() {
            if let Some((_, point)) = self.touch.take() {
                events.push(VitaInputEvent::Ui(InputEvent::PointerUp(point)));
            }
            self.touch_ready = true;
        } else if let Some((id, previous)) = self.touch {
            if let Some(report) = reports.iter().find(|r| r.id == id) {
                let point = self.point(*report);
                if point != previous {
                    events.push(VitaInputEvent::Ui(InputEvent::PointerMove(point)));
                }
                self.touch = Some((id, point));
            } else {
                self.touch = None;
                self.touch_ready = false;
                events.push(VitaInputEvent::CancelPointer);
            }
        } else if self.touch_ready {
            let report = reports[0];
            let point = self.point(report);
            self.touch = Some((report.id, point));
            events.push(VitaInputEvent::Ui(InputEvent::PointerDown(point)));
        }
        events
    }

    fn point(&self, report: SceTouchReport) -> Point {
        let [x, y] = self.panel.display_position(report);
        Point {
            x: x as f32,
            y: y as f32,
        }
    }
}

impl Drop for VitaInput {
    fn drop(&mut self) {
        unsafe {
            sceTouchSetSamplingState(SCE_TOUCH_PORT_FRONT, SCE_TOUCH_SAMPLING_STATE_STOP);
        }
    }
}

#[derive(Default)]
struct Repeat {
    held: u32,
    next: Option<Instant>,
}

impl Repeat {
    fn poll(&mut self, held: u32, now: Instant) -> u32 {
        if held != self.held {
            self.held = held;
            self.next = (held != 0).then_some(now + Duration::from_millis(350));
            return 0;
        }
        if self.next.is_some_and(|next| now >= next) {
            self.next = Some(now + Duration::from_millis(85));
            held
        } else {
            0
        }
    }
}

fn buttons() -> u32 {
    let mut pad = unsafe { std::mem::zeroed::<SceCtrlData>() };
    if unsafe { sceCtrlPeekBufferPositive(0, &mut pad, 1) } < 1 {
        return 0;
    }
    let mut buttons = pad.buttons;
    if pad.lx < 72 {
        buttons |= SCE_CTRL_LEFT;
    }
    if pad.lx > 184 {
        buttons |= SCE_CTRL_RIGHT;
    }
    if pad.ly < 72 {
        buttons |= SCE_CTRL_UP;
    }
    if pad.ly > 184 {
        buttons |= SCE_CTRL_DOWN;
    }
    buttons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_waits_then_repeats_without_catching_up_after_io() {
        let start = Instant::now();
        let mut repeat = Repeat::default();
        assert_eq!(repeat.poll(SCE_CTRL_DOWN, start), 0);
        assert_eq!(
            repeat.poll(SCE_CTRL_DOWN, start + Duration::from_millis(349)),
            0
        );
        assert_eq!(
            repeat.poll(SCE_CTRL_DOWN, start + Duration::from_millis(350)),
            SCE_CTRL_DOWN
        );
        assert_eq!(
            repeat.poll(SCE_CTRL_DOWN, start + Duration::from_secs(5)),
            SCE_CTRL_DOWN
        );
        assert_eq!(
            repeat.poll(SCE_CTRL_DOWN, start + Duration::from_secs(5)),
            0
        );
        assert_eq!(repeat.poll(0, start + Duration::from_secs(6)), 0);
        assert_eq!(
            repeat.poll(SCE_CTRL_DOWN, start + Duration::from_secs(6)),
            0
        );
    }
}

const DISPLAY_SIZE: [u32; 2] = [960, 544];
pub(crate) struct TouchPanel {
    min: [i32; 2],
    max: [i32; 2],
}

impl TouchPanel {
    pub(crate) fn front() -> Self {
        let mut info = unsafe { std::mem::zeroed::<SceTouchPanelInfo>() };
        if unsafe { sceTouchGetPanelInfo(SCE_TOUCH_PORT_FRONT, &mut info) } >= 0
            && info.maxDispX > info.minDispX
            && info.maxDispY > info.minDispY
        {
            Self {
                min: [i32::from(info.minDispX), i32::from(info.minDispY)],
                max: [i32::from(info.maxDispX), i32::from(info.maxDispY)],
            }
        } else {
            Self {
                min: [0, 0],
                max: [1919, 1087],
            }
        }
    }

    pub(crate) fn display_position(&self, report: SceTouchReport) -> [i32; 2] {
        [
            scale_axis(
                i32::from(report.x),
                self.min[0],
                self.max[0],
                DISPLAY_SIZE[0],
            ),
            scale_axis(
                i32::from(report.y),
                self.min[1],
                self.max[1],
                DISPLAY_SIZE[1],
            ),
        ]
    }
}

fn scale_axis(value: i32, min: i32, max: i32, output: u32) -> i32 {
    let range = (max - min + 1).max(1);
    (i64::from((value - min).clamp(0, range - 1)) * i64::from(output) / i64::from(range)) as i32
}
