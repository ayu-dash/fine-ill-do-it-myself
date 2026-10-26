/// Placeholder module until the real core logic lands in stage 1.
pub fn app_display_name() -> &'static str {
    "fine ill do it myself"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_display_name() {
        assert_eq!(app_display_name(), "fine ill do it myself");
    }
}
