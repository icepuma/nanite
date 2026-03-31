use inquire::ui::{Attributes, Color, RenderConfig, StyleSheet, Styled};

pub fn inquire_render_config() -> RenderConfig<'static> {
    RenderConfig {
        prompt_prefix: Styled::new("•")
            .with_fg(Color::LightGreen)
            .with_attr(Attributes::BOLD),
        answered_prompt_prefix: Styled::new("✓")
            .with_fg(Color::LightGreen)
            .with_attr(Attributes::BOLD),
        highlighted_option_prefix: Styled::new("›")
            .with_fg(Color::LightCyan)
            .with_attr(Attributes::BOLD),
        prompt: StyleSheet::new().with_attr(Attributes::BOLD),
        selected_option: Some(
            StyleSheet::new()
                .with_fg(Color::LightCyan)
                .with_attr(Attributes::BOLD),
        ),
        ..RenderConfig::default()
    }
}
