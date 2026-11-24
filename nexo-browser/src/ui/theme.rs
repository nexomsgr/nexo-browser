//! Цветовые схемы Nexo Browser — в одном стиле с мессенджером Nexo:
//! глубокий тёмно-синий градиент, скруглённые панели с тонкой рамкой,
//! синий акцент (аватары, выбранные элементы), голубые иконки, янтарная кнопка действия.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn css(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Линейная интерполяция между двумя цветами, `t` в диапазоне 0.0..=1.0.
    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Градиент фона окна: от верхнего левого к нижнему правому углу.
    pub bg_start: Rgb,
    pub bg_end: Rgb,
    /// Панели (панель вкладок, адресная строка, карточки) и их рамка.
    pub panel: Rgb,
    pub panel_border: Rgb,
    pub panel_hover: Rgb,
    /// Выбранный элемент (активная вкладка, выбранная строка).
    pub selected: Rgb,
    /// Основной синий акцент (кнопки, аватары, индикаторы).
    pub accent: Rgb,
    /// Голубой акцент для иконок и ссылок.
    pub accent_cyan: Rgb,
    /// Тёплый акцент для особого действия (как кнопка видео в мессенджере).
    pub warm: Rgb,
    pub warm_bg: Rgb,
    pub success: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
}

impl Theme {
    /// Цвет градиента фона в точке `t` (0.0 — левый верх, 1.0 — правый низ).
    pub fn background_at(&self, t: f32) -> Rgb {
        self.bg_start.lerp(self.bg_end, t)
    }

    /// CSS-переменные для внутренних страниц nexo://… и локального HTML-интерфейса.
    pub fn css_vars(&self) -> String {
        format!(
            ":root{{--bg-start:{};--bg-end:{};--panel:{};--panel-border:{};--panel-hover:{};\
             --selected:{};--accent:{};--accent-cyan:{};--warm:{};--warm-bg:{};--success:{};\
             --text:{};--text-dim:{};--radius:18px;}}\
             body{{background:linear-gradient(135deg,var(--bg-start),var(--bg-end));color:var(--text);}}",
            self.bg_start.css(),
            self.bg_end.css(),
            self.panel.css(),
            self.panel_border.css(),
            self.panel_hover.css(),
            self.selected.css(),
            self.accent.css(),
            self.accent_cyan.css(),
            self.warm.css(),
            self.warm_bg.css(),
            self.success.css(),
            self.text.css(),
            self.text_dim.css(),
        )
    }
}

/// Тёмная тема — основная, как в мессенджере.
pub const DARK: Theme = Theme {
    bg_start: Rgb(0x0A, 0x12, 0x20),
    bg_end: Rgb(0x0B, 0x1E, 0x3D),
    panel: Rgb(0x0E, 0x17, 0x28),
    panel_border: Rgb(0x1B, 0x2A, 0x45),
    panel_hover: Rgb(0x14, 0x21, 0x3A),
    selected: Rgb(0x15, 0x2A, 0x4F),
    accent: Rgb(0x16, 0x65, 0xD8),
    accent_cyan: Rgb(0x35, 0xB8, 0xF5),
    warm: Rgb(0xF2, 0xB1, 0x1A),
    warm_bg: Rgb(0x3A, 0x2F, 0x14),
    success: Rgb(0x2E, 0xCC, 0x71),
    text: Rgb(0xEA, 0xEF, 0xFA),
    text_dim: Rgb(0x8A, 0x98, 0xB3),
};

/// Светлая тема — тот же голубой характер, но на светлом градиенте.
pub const LIGHT: Theme = Theme {
    bg_start: Rgb(0xF2, 0xF7, 0xFF),
    bg_end: Rgb(0xDC, 0xE9, 0xFB),
    panel: Rgb(0xFF, 0xFF, 0xFF),
    panel_border: Rgb(0xD3, 0xDF, 0xF2),
    panel_hover: Rgb(0xEA, 0xF1, 0xFC),
    selected: Rgb(0xD6, 0xE6, 0xFF),
    accent: Rgb(0x16, 0x65, 0xD8),
    accent_cyan: Rgb(0x0A, 0x8F, 0xD6),
    warm: Rgb(0xC9, 0x8A, 0x00),
    warm_bg: Rgb(0xFF, 0xF1, 0xCC),
    success: Rgb(0x1E, 0xA8, 0x58),
    text: Rgb(0x14, 0x1C, 0x2E),
    text_dim: Rgb(0x5C, 0x6B, 0x86),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_hex_format() {
        assert_eq!(Rgb(0x0A, 0x12, 0x20).css(), "#0a1220");
        assert_eq!(Rgb(255, 255, 255).css(), "#ffffff");
    }

    #[test]
    fn gradient_endpoints_and_midpoint() {
        assert_eq!(DARK.background_at(0.0), DARK.bg_start);
        assert_eq!(DARK.background_at(1.0), DARK.bg_end);
        assert_eq!(Rgb(0, 0, 0).lerp(Rgb(100, 200, 50), 0.5), Rgb(50, 100, 25));
        // Значения вне диапазона не выходят за границы.
        assert_eq!(DARK.background_at(5.0), DARK.bg_end);
    }

    #[test]
    fn css_vars_contain_gradient_and_accents() {
        let css = DARK.css_vars();
        assert!(css.contains("--bg-start:#0a1220"));
        assert!(css.contains("linear-gradient(135deg"));
        assert!(css.contains("--accent:#1665d8"));
    }

    #[test]
    fn dark_is_darker_than_light() {
        let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
        assert!(lum(DARK.bg_start) < lum(LIGHT.bg_start));
        assert!(lum(DARK.text) > lum(DARK.bg_start));
    }
}
