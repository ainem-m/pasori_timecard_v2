pub(crate) fn card_scanned_message(_card_id: &str) -> &'static str {
    "card scanned"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // カードスキャンログ文言には full IDm を含めない。
    fn omits_full_idm_from_card_scanned_log_message() {
        let full_idm = "02020212A91B9843";

        let message = card_scanned_message(full_idm);

        assert_eq!(message, "card scanned");
        assert!(!message.contains(full_idm));
    }
}
