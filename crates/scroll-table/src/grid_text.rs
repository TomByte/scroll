//! Text the grid reads or shows besides values: clipboard blocks and the row count.

/// Spreadsheet-style clipboard text as rows of cells: lines separated by newlines, cells by tabs.
pub(crate) fn clipboard_grid(text: &str) -> Vec<Vec<String>> {
    text.trim_end_matches(['\n', '\r'])
        .split('\n')
        .map(|line| {
            line.trim_end_matches('\r')
                .split('\t')
                .map(str::to_string)
                .collect()
        })
        .collect()
}

/// "1,234 rows".
pub(crate) fn row_count_label(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let noun = if count == 1 { "row" } else { "rows" };
    format!("{grouped} {noun}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_text_splits_into_rows_and_cells() {
        assert_eq!(
            clipboard_grid("a\tb\r\nc\n"),
            vec![vec!["a", "b"], vec!["c"]]
        );
        assert_eq!(clipboard_grid("single"), vec![vec!["single"]]);
    }

    #[test]
    fn row_counts_group_thousands() {
        assert_eq!(row_count_label(1), "1 row");
        assert_eq!(row_count_label(0), "0 rows");
        assert_eq!(row_count_label(100_000), "100,000 rows");
        assert_eq!(row_count_label(1_234_567), "1,234,567 rows");
    }
}
