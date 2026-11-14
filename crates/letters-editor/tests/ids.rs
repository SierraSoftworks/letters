use letters_editor::block_id;

#[test]
fn a_block_id_spells_all_of_its_random_bits_in_sixteen_plain_characters() {
    let cases = [[0; 10], [255; 10], [1, 2, 3, 4, 5, 6, 7, 8, 9, 10], [1, 2, 3, 4, 5, 6, 7, 8, 9, 11]];
    let ids = cases.map(block_id);
    for id in &ids {
        assert_eq!(id.len(), 16, "{id}");
        assert!(id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{id}");
    }
    for (at, id) in ids.iter().enumerate() {
        assert!(!ids[..at].contains(id), "{id} names two different sets of bits");
    }
}
