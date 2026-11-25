//! "Capture windows separately": which windows may be photographed on their own, and why not.
//!
//! AIDEV-NOTE: privacy-critical. Pure policy, no screen needed. It may only be stricter than whole-display capture:
//! AIDEV-NOTE: every window is refused unless every rule lets it through, and what can't be told counts as a refusal.
//! AIDEV-NOTE: A window of an app that shows pages is taken only when it is the one in front, because the capture
//! AIDEV-NOTE: check vouches for a browser as a whole and only the front window's address is known for certain.

use crate::detect::Rect;
use crate::privacy::{Exclusions, hide_app, might_show_pages};

/// The most background windows looked at in one capture, nearest the front first. Keeps the time between the
/// before and after checks short.
pub const MAX_BACKGROUND: usize = 8;

/// One window as the window server lists it, front to back.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowSeen {
    pub id: u32,
    pub pid: i32,
    /// Empty when nothing says which app this is.
    pub bundle_id: String,
    /// An ordinary app, as opposed to a helper or an agent that has no place in the Dock.
    pub regular: bool,
    pub layer: i32,
    pub alpha: f64,
    /// Drawn on this Space: not minimised, not hidden, not on another desktop. It may be wholly covered.
    pub on_screen: bool,
    /// Where it is on the desktop, in points.
    pub bounds: Rect,
}

/// What bears on every window at one instant.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowScene {
    pub own_pid: i32,
    pub front_pid: i32,
    pub front_bundle_id: String,
    /// The window a person would say they're in, if the app in front has one.
    pub front_window: Option<u32>,
    /// Apps the capture check found off-limits just now (excluded, a private window, an address it couldn't read).
    pub hidden: Vec<i32>,
    /// The displays a picture may come from, in the same points as window bounds. A window must lie mostly on them.
    pub desktop: Vec<Rect>,
}

/// Why a window isn't photographed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum WindowSkip {
    /// Afterimage's own window: it shows other pictures.
    Ours,
    /// Its app is excluded, a password manager or a system prompt, or the capture check hid it.
    Hidden,
    /// Something off-limits is in front, so nothing is looked at.
    PrivateInFront,
    /// Nothing says which app owns it.
    Unknown,
    /// A window of an app that shows pages, and it isn't the one in front.
    BackgroundPages,
    /// Minimised, hidden, faint, on another desktop, or mostly off the displays being captured.
    NotShown,
    /// A menu, a panel or an overlay.
    NotAWindow,
    /// A helper's window in the background.
    NotAnApp,
    TooSmall,
    /// More background windows than one capture looks at.
    TooMany,
}

/// One window and the verdict on it. `skip` of None means it may be photographed.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowChoice {
    pub id: u32,
    pub pid: i32,
    /// It is the window in front.
    pub front: bool,
    pub bounds: Rect,
    pub skip: Option<WindowSkip>,
}

/// Decide, window by window, what may be photographed. `windows` is front to back; the answer is in the same order.
#[uniffi::export]
pub fn plan_windows(ex: &Exclusions, scene: &WindowScene, windows: &[WindowSeen]) -> Vec<WindowChoice> {
    // Whole-display capture takes nothing when something off-limits is in front. Neither does this.
    let private_front = scene.front_pid == scene.own_pid || scene.hidden.contains(&scene.front_pid) || hide_app(ex, &scene.front_bundle_id).is_some();
    let mut background = 0;
    // Mirrored displays are the same place listed more than once; counted once.
    let finite = |r: &Rect| [r.x, r.y, r.w, r.h].iter().all(|n| n.is_finite());
    let inside = |a: &Rect, b: &Rect| a.x >= b.x && a.y >= b.y && a.x + a.w <= b.x + b.w && a.y + a.h <= b.y + b.h;
    let mut desktop: Vec<Rect> = Vec::new();
    for d in scene.desktop.iter().filter(|d| finite(d)) {
        if !desktop.iter().any(|k| inside(d, k)) {
            desktop.retain(|k| !inside(k, d));
            desktop.push(*d);
        }
    }
    windows
        .iter()
        .map(|w| {
            let front = w.pid == scene.front_pid && scene.front_window == Some(w.id);
            let sized = w.bounds.w.is_finite() && w.bounds.h.is_finite() && w.bounds.w > 160.0 && w.bounds.h > 160.0;
            // AIDEV-NOTE: privacy-critical. "Listed as on screen" isn't "visible": tiling managers park windows past the edge.
            let on: f64 = desktop.iter().map(|d| {
                let x = (w.bounds.x + w.bounds.w).min(d.x + d.w) - w.bounds.x.max(d.x);
                let y = (w.bounds.y + w.bounds.h).min(d.y + d.h) - w.bounds.y.max(d.y);
                if x > 0.0 && y > 0.0 { x * y } else { 0.0 }
            }).sum();
            let visible = sized && finite(&w.bounds) && on >= 0.5 * w.bounds.w * w.bounds.h && w.alpha >= 0.5;
            let skip = if w.pid == scene.own_pid {
                Some(WindowSkip::Ours)
            } else if scene.hidden.contains(&w.pid) || hide_app(ex, &w.bundle_id).is_some() {
                Some(WindowSkip::Hidden)
            } else if private_front {
                Some(WindowSkip::PrivateInFront)
            } else if w.bundle_id.trim().is_empty() {
                Some(WindowSkip::Unknown)
            } else if !front && might_show_pages(&w.bundle_id) {
                Some(WindowSkip::BackgroundPages)
            } else if !w.on_screen || !(w.alpha > 0.0) {
                Some(WindowSkip::NotShown)
            } else if w.layer != 0 {
                Some(WindowSkip::NotAWindow)
            } else if !sized {
                Some(WindowSkip::TooSmall)
            } else if !visible {
                Some(WindowSkip::NotShown)
            } else if !front && !w.regular {
                Some(WindowSkip::NotAnApp)
            } else if !front && background >= MAX_BACKGROUND {
                Some(WindowSkip::TooMany)
            } else {
                background += usize::from(!front);
                None
            };
            WindowChoice { id: w.id, pid: w.pid, front, bounds: w.bounds, skip }
        })
        .collect()
}

/// The windows photographed between two looks that may be kept: allowed both before and after, by the same app, in
/// the same role and in the same place. Anything that changed in between is let go.
#[uniffi::export]
pub fn keep_windows(before: &[WindowChoice], after: &[WindowChoice]) -> Vec<u32> {
    let near = |a: f64, b: f64| (a - b).abs() <= 1.0;
    let same = |a: &Rect, b: &Rect| near(a.x, b.x) && near(a.y, b.y) && near(a.w, b.w) && near(a.h, b.h);
    before
        .iter()
        .filter(|b| b.skip.is_none())
        .filter(|b| {
            // Falling off the end of a longer list is no reason to doubt a picture already taken.
            let mut now = after.iter().filter(|a| a.id == b.id);
            matches!((now.next(), now.next()), (Some(a), None) if a.pid == b.pid && a.front == b.front && same(&a.bounds, &b.bounds)
                && matches!(a.skip, None | Some(WindowSkip::TooMany)))
        })
        .map(|b| b.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: i32 = 1;
    const XCODE: &str = "com.apple.dt.Xcode";
    const NOTES: &str = "com.apple.Notes";
    const CHROME: &str = "com.google.Chrome";

    fn ex() -> Exclusions {
        Exclusions { apps: vec!["com.acme.Ledger".into()], sites: vec![], password_managers: true, private_windows: true, unreadable_browsers: true, tor: true }
    }
    fn win(id: u32, pid: i32, bundle: &str) -> WindowSeen {
        WindowSeen { id, pid, bundle_id: bundle.into(), regular: true, layer: 0, alpha: 1.0, on_screen: true, bounds: Rect { x: 10.0, y: 20.0, w: 900.0, h: 600.0 } }
    }
    fn scene(front_pid: i32, bundle: &str, window: Option<u32>) -> WindowScene {
        WindowScene { own_pid: ME, front_pid, front_bundle_id: bundle.into(), front_window: window, hidden: vec![ME], desktop: vec![Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 }, Rect { x: 1440.0, y: 0.0, w: 1920.0, h: 1080.0 }] }
    }
    /// Whole-display capture can only show what is on a display. A window parked past the edge, or all but
    /// invisible, is something the person can't see and it never photographed.
    #[test]
    fn a_window_the_person_cannot_see_is_not_photographed() {
        let s = scene(7, "com.editor", Some(1));
        let at = |x: f64, y: f64| { let mut w = win(2, 8, "com.notes"); w.bounds = Rect { x, y, w: 900.0, h: 600.0 }; w };
        let skip = |w: WindowSeen| plan_windows(&ex(), &s, &[win(1, 7, "com.editor"), w])[1].skip;
        // A sliver left on the desktop, the rest parked off it.
        assert_eq!(skip(at(-899.0, 20.0)), Some(WindowSkip::NotShown));
        assert_eq!(skip(at(100.0, 890.0)), Some(WindowSkip::NotShown));
        assert_eq!(skip(at(50_000.0, 50_000.0)), Some(WindowSkip::NotShown));
        // Mostly on: taken. Straddling two displays counts both.
        assert_eq!(skip(at(-300.0, 20.0)), None);
        assert_eq!(skip(at(1000.0, 20.0)), None);
        // Faint enough to be a trick rather than a window.
        let mut faint = win(2, 8, "com.notes");
        faint.alpha = 0.3;
        assert_eq!(skip(faint), Some(WindowSkip::NotShown));
        let mut sheer = win(2, 8, "com.notes");
        sheer.alpha = 0.9;
        assert_eq!(skip(sheer), None);
        // The window in front is held to the same rule.
        let mut f = win(1, 7, "com.editor");
        f.bounds.x = -899.0;
        assert_eq!(plan_windows(&ex(), &s, &[f])[0].skip, Some(WindowSkip::NotShown));
        // Mirrored displays are one place listed twice: a sliver on it doesn't count double.
        let mirrored = WindowScene { desktop: vec![Rect { x: 0.0, y: 0.0, w: 1440.0, h: 900.0 }; 3], ..s.clone() };
        let mut third = win(2, 8, "com.notes");
        third.bounds.x = -600.0;
        assert_eq!(plan_windows(&ex(), &mirrored, &[win(1, 7, "com.editor"), third])[1].skip, Some(WindowSkip::NotShown));
        let mut nowhere = win(2, 8, "com.notes");
        nowhere.bounds.x = f64::NAN;
        assert_eq!(plan_windows(&ex(), &s, &[win(1, 7, "com.editor"), nowhere])[1].skip, Some(WindowSkip::NotShown));
        // Nothing known about the displays: nothing can be shown to be on one.
        let blind = WindowScene { desktop: vec![], ..s.clone() };
        assert_eq!(plan_windows(&ex(), &blind, &[win(1, 7, "com.editor")])[0].skip, Some(WindowSkip::NotShown));
    }

    fn verdicts(s: &WindowScene, w: &[WindowSeen]) -> Vec<(u32, bool, Option<WindowSkip>)> {
        plan_windows(&ex(), s, w).into_iter().map(|c| (c.id, c.front, c.skip)).collect()
    }

    #[test]
    fn the_front_window_and_ordinary_background_windows_are_each_their_own_picture() {
        let w = [win(7, 10, XCODE), win(8, 10, XCODE), win(9, 11, NOTES)];
        assert_eq!(verdicts(&scene(10, XCODE, Some(7)), &w), vec![(7, true, None), (8, false, None), (9, false, None)]);
    }

    #[test]
    fn an_excluded_app_is_never_photographed_front_or_back() {
        for b in ["com.acme.ledger", "com.bitwarden.desktop", "com.apple.SecurityAgent", "org.torproject.torbrowser"] {
            // In the background: its windows are refused and everything else carries on.
            let w = [win(7, 10, XCODE), win(8, 12, b)];
            assert_eq!(verdicts(&scene(10, XCODE, Some(7)), &w), vec![(7, true, None), (8, false, Some(WindowSkip::Hidden))], "{b}");
            // In front: nothing at all is looked at, exactly as with a whole display.
            let w = [win(8, 12, b), win(7, 10, XCODE)];
            assert_eq!(verdicts(&scene(12, b, Some(8)), &w), vec![(8, true, Some(WindowSkip::Hidden)), (7, false, Some(WindowSkip::PrivateInFront))], "{b}");
        }
    }

    #[test]
    fn what_the_capture_check_hid_is_never_photographed() {
        // A browser with a private window, or an address it couldn't read: the check names the app, not the window.
        let mut s = scene(10, XCODE, Some(7));
        s.hidden = vec![ME, 13];
        let w = [win(7, 10, XCODE), win(8, 13, "com.example.viewer")];
        assert_eq!(verdicts(&s, &w)[1], (8, false, Some(WindowSkip::Hidden)));
        // And when it is the one in front, nothing is taken.
        let mut s = scene(13, "com.example.viewer", Some(8));
        s.hidden = vec![ME, 13];
        assert!(plan_windows(&ex(), &s, &w).iter().all(|c| c.skip.is_some()));
    }

    #[test]
    fn a_browser_window_is_taken_only_when_it_is_the_one_in_front() {
        let w = [win(7, 10, XCODE), win(8, 14, CHROME), win(9, 14, CHROME)];
        // In the background: never, however many windows it has.
        assert_eq!(verdicts(&scene(10, XCODE, Some(7)), &w), vec![(7, true, None), (8, false, Some(WindowSkip::BackgroundPages)), (9, false, Some(WindowSkip::BackgroundPages))]);
        // In front: its front window alone. Its other windows are background windows of a browser.
        let w = [win(8, 14, CHROME), win(9, 14, CHROME), win(7, 10, XCODE)];
        assert_eq!(verdicts(&scene(14, CHROME, Some(8)), &w), vec![(8, true, None), (9, false, Some(WindowSkip::BackgroundPages)), (7, false, None)]);
        // Every kind of browser, and a browser nobody has heard of that this Mac says opens pages.
        crate::privacy::register_browsers(vec!["com.example.windows-test-browser".into()]);
        for b in ["com.apple.Safari", "org.mozilla.firefox", "company.thebrowser.Browser", "com.microsoft.edgemac.Dev", "com.example.windows-test-browser"] {
            assert_eq!(verdicts(&scene(10, XCODE, Some(7)), &[win(7, 10, XCODE), win(8, 15, b)])[1].2, Some(WindowSkip::BackgroundPages), "{b}");
        }
    }

    #[test]
    fn a_window_in_front_of_another_app_is_not_this_apps_front_window() {
        // Window ids are the window server's; the same number under another app is not the window in front.
        let w = [win(7, 11, CHROME)];
        assert_eq!(verdicts(&scene(10, XCODE, Some(7)), &w), vec![(7, false, Some(WindowSkip::BackgroundPages))]);
    }

    #[test]
    fn afterimage_itself_is_never_photographed() {
        let w = [win(5, ME, "com.jmaddington.afterimage"), win(7, 10, XCODE)];
        assert_eq!(verdicts(&scene(10, XCODE, Some(7)), &w)[0], (5, false, Some(WindowSkip::Ours)));
        // In front, it stops everything.
        assert!(plan_windows(&ex(), &scene(ME, "com.jmaddington.afterimage", Some(5)), &w).iter().all(|c| c.skip.is_some()));
        // Even if the list of hidden apps came without it.
        let mut s = scene(10, XCODE, Some(7));
        s.hidden = vec![];
        assert_eq!(verdicts(&s, &w)[0].2, Some(WindowSkip::Ours));
        let mut s = scene(ME, "com.jmaddington.afterimage", Some(5));
        s.hidden = vec![];
        assert_eq!(verdicts(&s, &w), vec![(5, true, Some(WindowSkip::Ours)), (7, false, Some(WindowSkip::PrivateInFront))]);
    }

    #[test]
    fn minimised_hidden_and_see_through_windows_are_skipped() {
        let mut gone = win(8, 11, NOTES);
        gone.on_screen = false;
        let mut clear = win(9, 11, NOTES);
        clear.alpha = 0.0;
        let mut odd = win(10, 11, NOTES);
        odd.alpha = f64::NAN;
        let v = verdicts(&scene(10, XCODE, Some(7)), &[win(7, 10, XCODE), gone.clone(), clear, odd]);
        assert_eq!(v[1..].iter().map(|x| x.2).collect::<Vec<_>>(), vec![Some(WindowSkip::NotShown); 3]);
        // Being the front window doesn't change that.
        assert_eq!(verdicts(&scene(11, NOTES, Some(8)), &[gone])[0], (8, true, Some(WindowSkip::NotShown)));
    }

    #[test]
    fn menus_overlays_slivers_and_ownerless_windows_are_skipped() {
        let mut menu = win(8, 11, NOTES);
        menu.layer = 101;
        let mut sliver = win(9, 11, NOTES);
        sliver.bounds.h = 22.0;
        let mut senseless = win(10, 11, NOTES);
        senseless.bounds.w = f64::INFINITY;
        let nobody = win(11, 16, " ");
        let mut helper = win(12, 17, "com.example.agent");
        helper.regular = false;
        let v = verdicts(&scene(10, XCODE, Some(7)), &[win(7, 10, XCODE), menu, sliver, senseless, nobody.clone(), helper.clone()]);
        assert_eq!(v[1..].iter().map(|x| x.2).collect::<Vec<_>>(),
                   vec![Some(WindowSkip::NotAWindow), Some(WindowSkip::TooSmall), Some(WindowSkip::TooSmall), Some(WindowSkip::Unknown), Some(WindowSkip::NotAnApp)]);
        // A helper's window is taken when the person is actually in it; an ownerless one never is.
        assert_eq!(verdicts(&scene(17, "com.example.agent", Some(12)), &[helper])[0], (12, true, None));
        assert_eq!(verdicts(&scene(16, "", Some(11)), &[nobody])[0], (11, true, Some(WindowSkip::Unknown)));
    }

    #[test]
    fn only_so_many_background_windows_are_looked_at_and_the_nearest_come_first() {
        let mut w = vec![win(100, 10, XCODE)];
        w.extend((0..MAX_BACKGROUND as u32 + 3).map(|i| win(200 + i, 11, NOTES)));
        // A refused window in between doesn't use up a place.
        w.insert(2, win(150, 14, CHROME));
        let v = verdicts(&scene(10, XCODE, Some(100)), &w);
        assert_eq!(v.iter().filter(|x| x.2.is_none()).count(), 1 + MAX_BACKGROUND);
        assert_eq!(v.iter().filter(|x| x.2 == Some(WindowSkip::TooMany)).map(|x| x.0).collect::<Vec<_>>(), vec![208, 209, 210]);
        assert_eq!(v[0], (100, true, None));
    }

    #[test]
    fn a_new_exclusion_can_only_take_windows_away() {
        let w = [win(7, 10, XCODE), win(8, 11, NOTES), win(9, 14, CHROME)];
        let s = scene(10, XCODE, Some(7));
        let open = Exclusions { apps: vec![], sites: vec![], password_managers: false, private_windows: false, unreadable_browsers: false, tor: false };
        let loose: Vec<u32> = plan_windows(&open, &s, &w).into_iter().filter(|c| c.skip.is_none()).map(|c| c.id).collect();
        let strict = Exclusions { apps: vec![NOTES.into()], ..open };
        let tight: Vec<u32> = plan_windows(&strict, &s, &w).into_iter().filter(|c| c.skip.is_none()).map(|c| c.id).collect();
        assert_eq!((loose, tight), (vec![7, 8], vec![7]));
    }

    #[test]
    fn a_picture_is_kept_only_if_nothing_about_its_window_changed_meanwhile() {
        let w = [win(7, 10, XCODE), win(8, 11, NOTES), win(9, 14, CHROME)];
        let s = scene(10, XCODE, Some(7));
        let before = plan_windows(&ex(), &s, &w);
        assert_eq!(keep_windows(&before, &before), vec![7, 8]);

        // Its app became off-limits while the picture was being taken.
        let mut hid = s.clone();
        hid.hidden.push(11);
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &hid, &w)), vec![7]);
        // It closed, or was minimised.
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &w[..1])), vec![7]);
        let mut gone = w.clone();
        gone[1].on_screen = false;
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &gone)), vec![7]);
        // It moved or changed size: what was placed on it may no longer be where it was.
        let mut moved = w.clone();
        moved[0].bounds.x += 40.0;
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &moved)), vec![8]);
        for change in [|r: &mut Rect| r.y += 40.0, |r: &mut Rect| r.w -= 40.0, |r: &mut Rect| r.h -= 40.0] {
            let mut m = w.clone();
            change(&mut m[0].bounds);
            assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &m)), vec![8]);
        }
        let mut jitter = w.clone();
        jitter[0].bounds.x += 0.5;
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &jitter)), vec![7, 8]);
        // Another window came to the front: the one that was in front isn't what it was taken as.
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &scene(10, XCODE, Some(70)), &w)), vec![8]);
        // The number now belongs to another app.
        let mut reused = w.clone();
        reused[1].pid = 99;
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &reused)), vec![7]);
        // Listed twice afterwards: no telling which is meant.
        let twice = [w[0].clone(), w[1].clone(), w[1].clone()];
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &s, &twice)), vec![7]);
        // Refused before the picture and allowed after: it was never photographed, and can't be kept.
        assert_eq!(keep_windows(&plan_windows(&ex(), &hid, &w), &before), vec![7]);
        // Something private came to the front.
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &scene(12, "com.acme.Ledger", Some(3)), &w)), Vec::<u32>::new());
    }

    #[test]
    fn a_browser_in_front_is_dropped_when_it_stops_being_in_front() {
        let w = [win(8, 14, CHROME), win(7, 10, XCODE)];
        let before = plan_windows(&ex(), &scene(14, CHROME, Some(8)), &w);
        assert_eq!(keep_windows(&before, &before), vec![8, 7]);
        assert_eq!(keep_windows(&before, &plan_windows(&ex(), &scene(10, XCODE, Some(7)), &w)), Vec::<u32>::new());
    }

    #[test]
    fn being_pushed_off_the_end_of_the_list_does_not_lose_a_picture() {
        let mut w = vec![win(100, 10, XCODE)];
        w.extend((0..MAX_BACKGROUND as u32).map(|i| win(200 + i, 11, NOTES)));
        let s = scene(10, XCODE, Some(100));
        let before = plan_windows(&ex(), &s, &w);
        let mut more = w.clone();
        more.insert(1, win(300, 11, NOTES));
        let kept = keep_windows(&before, &plan_windows(&ex(), &s, &more));
        assert_eq!(kept.len(), 1 + MAX_BACKGROUND);
    }
}
