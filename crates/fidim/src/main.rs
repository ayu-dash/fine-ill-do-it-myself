fn main() {
    println!("{}", fine_ill_do_it_myself::app_display_name());
}

#[cfg(test)]
mod tests {
    #[test]
    fn prints_display_name() {
        let out = fine_ill_do_it_myself::app_display_name();
        assert!(out.contains("fine ill do it myself"));
    }
}
