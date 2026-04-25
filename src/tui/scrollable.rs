use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 3ペイン共通スクロール・画面移動 trait。
///
/// `move_one_*` の「1単位」は実装依存:
/// - Editor: 1文字 / 1行
/// - Results: 4セル / 1行
/// - Schema: 1行 / 横は no-op
///
/// 横方向操作は実装によって no-op を許容する（Schema は横スクロール状態を持たない）。
pub trait Scrollable {
    fn move_one_down(&mut self);
    fn move_one_up(&mut self);
    fn move_one_left(&mut self);
    fn move_one_right(&mut self);
    fn scroll_to_top(&mut self);
    fn scroll_to_bottom(&mut self);
    fn h_scroll_home(&mut self);
    fn h_scroll_end(&mut self);
    fn page_down(&mut self, page_size: usize);
    fn page_up(&mut self, page_size: usize);
    fn h_page_left(&mut self);
    fn h_page_right(&mut self);
    /// vim の `zz` 相当: カーソル行が画面中央に来るようビューを再センタリングする。
    /// `page_size` は呼び出し側が見積もる縦の表示行数で、半分の値が中央位置の目安となる。
    fn center_on_cursor(&mut self, page_size: usize);
}

/// `KeyEvent` を共通スクロールキーとして処理する。
///
/// 戻り値:
/// - `true`  : 共通キーとして処理した（呼び出し側は早期 return すべき）
/// - `false` : 未対応キー（呼び出し側のペイン固有処理に fallthrough する）
///
/// `S: ?Sized` は trait object（`&mut dyn Scrollable`）でも呼び出せるようにするための制約緩和。
pub fn dispatch_scroll_key<S: Scrollable + ?Sized>(
    s: &mut S,
    key: &KeyEvent,
    page_size: usize,
) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match (key.code, ctrl) {
        // 縦移動（1単位）
        (KeyCode::Char('j'), false) | (KeyCode::Down, false) => {
            s.move_one_down();
            true
        }
        (KeyCode::Char('k'), false) | (KeyCode::Up, false) => {
            s.move_one_up();
            true
        }
        // 横移動（1単位）
        (KeyCode::Char('h'), false) | (KeyCode::Left, false) => {
            s.move_one_left();
            true
        }
        (KeyCode::Char('l'), false) | (KeyCode::Right, false) => {
            s.move_one_right();
            true
        }
        // 縦先頭/末尾
        (KeyCode::Char('g'), false) => {
            s.scroll_to_top();
            true
        }
        (KeyCode::Char('G'), false) => {
            s.scroll_to_bottom();
            true
        }
        // 横先頭/末尾
        (KeyCode::Char('0'), false) | (KeyCode::Home, false) => {
            s.h_scroll_home();
            true
        }
        (KeyCode::Char('$'), false) | (KeyCode::End, false) => {
            s.h_scroll_end();
            true
        }
        // 縦ページ
        (KeyCode::PageDown, _) => {
            s.page_down(page_size);
            true
        }
        (KeyCode::PageUp, _) => {
            s.page_up(page_size);
            true
        }
        (KeyCode::Char('d'), true) => {
            s.page_down(page_size);
            true
        }
        (KeyCode::Char('u'), true) => {
            s.page_up(page_size);
            true
        }
        // 横ページ
        (KeyCode::Char('H'), false) => {
            s.h_page_left();
            true
        }
        (KeyCode::Char('L'), false) => {
            s.h_page_right();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// 呼び出されたメソッドを記録するダミー Scrollable 実装。
    /// dispatch が正しいメソッドへ委譲しているかを観測するためだけに使う。
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Recorder {
        events: Vec<String>,
    }

    impl Scrollable for Recorder {
        fn move_one_down(&mut self) {
            self.events.push("move_one_down".into());
        }
        fn move_one_up(&mut self) {
            self.events.push("move_one_up".into());
        }
        fn move_one_left(&mut self) {
            self.events.push("move_one_left".into());
        }
        fn move_one_right(&mut self) {
            self.events.push("move_one_right".into());
        }
        fn scroll_to_top(&mut self) {
            self.events.push("scroll_to_top".into());
        }
        fn scroll_to_bottom(&mut self) {
            self.events.push("scroll_to_bottom".into());
        }
        fn h_scroll_home(&mut self) {
            self.events.push("h_scroll_home".into());
        }
        fn h_scroll_end(&mut self) {
            self.events.push("h_scroll_end".into());
        }
        fn page_down(&mut self, page_size: usize) {
            self.events.push(format!("page_down:{}", page_size));
        }
        fn page_up(&mut self, page_size: usize) {
            self.events.push(format!("page_up:{}", page_size));
        }
        fn h_page_left(&mut self) {
            self.events.push("h_page_left".into());
        }
        fn h_page_right(&mut self) {
            self.events.push("h_page_right".into());
        }
        fn center_on_cursor(&mut self, page_size: usize) {
            self.events.push(format!("center_on_cursor:{}", page_size));
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn dispatch(rec: &mut Recorder, code: KeyCode) -> bool {
        dispatch_scroll_key(rec, &key(code), 20)
    }

    // ── 縦1単位 ──

    #[test]
    fn dispatch_char_j_calls_move_one_down() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('j'));
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_down".to_string()]);
    }

    #[test]
    fn dispatch_down_arrow_calls_move_one_down() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Down);
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_down".to_string()]);
    }

    #[test]
    fn dispatch_char_k_calls_move_one_up() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('k'));
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_up".to_string()]);
    }

    #[test]
    fn dispatch_up_arrow_calls_move_one_up() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Up);
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_up".to_string()]);
    }

    // ── 横1単位 ──

    #[test]
    fn dispatch_char_h_calls_move_one_left() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('h'));
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_left".to_string()]);
    }

    #[test]
    fn dispatch_left_arrow_calls_move_one_left() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Left);
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_left".to_string()]);
    }

    #[test]
    fn dispatch_char_l_calls_move_one_right() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('l'));
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_right".to_string()]);
    }

    #[test]
    fn dispatch_right_arrow_calls_move_one_right() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Right);
        assert!(handled);
        assert_eq!(rec.events, vec!["move_one_right".to_string()]);
    }

    // ── 縦先頭/末尾 ──

    #[test]
    fn dispatch_char_g_calls_scroll_to_top() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('g'));
        assert!(handled);
        assert_eq!(rec.events, vec!["scroll_to_top".to_string()]);
    }

    #[test]
    fn dispatch_char_capital_g_calls_scroll_to_bottom() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('G'));
        assert!(handled);
        assert_eq!(rec.events, vec!["scroll_to_bottom".to_string()]);
    }

    // ── 横先頭/末尾 ──

    #[test]
    fn dispatch_char_zero_calls_h_scroll_home() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('0'));
        assert!(handled);
        assert_eq!(rec.events, vec!["h_scroll_home".to_string()]);
    }

    #[test]
    fn dispatch_home_calls_h_scroll_home() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Home);
        assert!(handled);
        assert_eq!(rec.events, vec!["h_scroll_home".to_string()]);
    }

    #[test]
    fn dispatch_char_dollar_calls_h_scroll_end() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('$'));
        assert!(handled);
        assert_eq!(rec.events, vec!["h_scroll_end".to_string()]);
    }

    #[test]
    fn dispatch_end_calls_h_scroll_end() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::End);
        assert!(handled);
        assert_eq!(rec.events, vec!["h_scroll_end".to_string()]);
    }

    // ── 縦ページ ──

    #[test]
    fn dispatch_page_down_calls_page_down_with_page_size() {
        let mut rec = Recorder::default();
        let handled = dispatch_scroll_key(&mut rec, &key(KeyCode::PageDown), 20);
        assert!(handled);
        assert_eq!(rec.events, vec!["page_down:20".to_string()]);
    }

    #[test]
    fn dispatch_page_up_calls_page_up_with_page_size() {
        let mut rec = Recorder::default();
        let handled = dispatch_scroll_key(&mut rec, &key(KeyCode::PageUp), 20);
        assert!(handled);
        assert_eq!(rec.events, vec!["page_up:20".to_string()]);
    }

    #[test]
    fn dispatch_ctrl_d_calls_page_down() {
        let mut rec = Recorder::default();
        let event = key_with(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let handled = dispatch_scroll_key(&mut rec, &event, 20);
        assert!(handled);
        assert_eq!(rec.events, vec!["page_down:20".to_string()]);
    }

    #[test]
    fn dispatch_ctrl_u_calls_page_up() {
        let mut rec = Recorder::default();
        let event = key_with(KeyCode::Char('u'), KeyModifiers::CONTROL);
        let handled = dispatch_scroll_key(&mut rec, &event, 20);
        assert!(handled);
        assert_eq!(rec.events, vec!["page_up:20".to_string()]);
    }

    #[test]
    fn dispatch_page_down_passes_custom_page_size() {
        let mut rec = Recorder::default();
        let handled = dispatch_scroll_key(&mut rec, &key(KeyCode::PageDown), 7);
        assert!(handled);
        assert_eq!(rec.events, vec!["page_down:7".to_string()]);
    }

    // ── 横ページ ──

    #[test]
    fn dispatch_char_capital_h_calls_h_page_left() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('H'));
        assert!(handled);
        assert_eq!(rec.events, vec!["h_page_left".to_string()]);
    }

    #[test]
    fn dispatch_char_capital_l_calls_h_page_right() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('L'));
        assert!(handled);
        assert_eq!(rec.events, vec!["h_page_right".to_string()]);
    }

    // ── ctrl=false の d/u は dispatch しない（fallthrough） ──

    #[test]
    fn dispatch_plain_d_returns_false() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('d'));
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    #[test]
    fn dispatch_plain_u_returns_false() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('u'));
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    // ── 未対応キーは false ──

    #[test]
    fn dispatch_char_z_returns_false() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('z'));
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    #[test]
    fn dispatch_char_y_returns_false() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('y'));
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    #[test]
    fn dispatch_char_c_returns_false() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Char('c'));
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    #[test]
    fn dispatch_enter_returns_false() {
        let mut rec = Recorder::default();
        let handled = dispatch(&mut rec, KeyCode::Enter);
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    // ── ctrl 修飾子の境界 ──

    #[test]
    fn dispatch_ctrl_j_returns_false() {
        // Ctrl+j は dispatch 対象外（プレーン j のみ受ける）
        let mut rec = Recorder::default();
        let event = key_with(KeyCode::Char('j'), KeyModifiers::CONTROL);
        let handled = dispatch_scroll_key(&mut rec, &event, 20);
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    #[test]
    fn dispatch_ctrl_g_returns_false() {
        // Ctrl+g は dispatch 対象外
        let mut rec = Recorder::default();
        let event = key_with(KeyCode::Char('g'), KeyModifiers::CONTROL);
        let handled = dispatch_scroll_key(&mut rec, &event, 20);
        assert!(!handled);
        assert!(rec.events.is_empty());
    }

    #[test]
    fn dispatch_page_down_with_ctrl_still_dispatches() {
        // PageDown は ctrl 不問
        let mut rec = Recorder::default();
        let event = key_with(KeyCode::PageDown, KeyModifiers::CONTROL);
        let handled = dispatch_scroll_key(&mut rec, &event, 20);
        assert!(handled);
        assert_eq!(rec.events, vec!["page_down:20".to_string()]);
    }
}
