//! TUI のあいだ端末を raw mode・代替画面・bracketed paste にし、抜けるときに元へ戻す。

use crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;

/// 端末を TUI 用の状態にし、`restore` か Drop で元に戻すガード。
///
/// メインループがエラーで抜けて `restore` を呼ばずに捨てられても、Drop で端末を戻す。
/// panic のときは `run` の panic hook が先に戻す。hook がメッセージを出すのは unwind の前なので、
/// Drop を待つとメッセージが代替画面に出て消える。
pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        // ここから先で失敗しても、guard の Drop で raw mode を戻す
        let guard = Self { active: true };
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        Ok(guard)
    }

    /// 端末を戻す。片方が失敗しても残りを続け、最初のエラーを返す。
    ///
    /// stdout に書けない（EPIPE 等）ときも raw mode は戻せるよう、エスケープシーケンスの書き込みとは分けている。
    pub fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        let raw_mode = disable_raw_mode();
        let screen = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show
        );
        raw_mode.and(screen)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
