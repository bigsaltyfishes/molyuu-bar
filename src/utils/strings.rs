pub trait StringExt {
    /// Checks if a substring of the string is equal to another string.
    /// 
    /// # Arguments
    /// 
    /// * `other` - The string to compare against.
    /// * `begin` - The starting index of the substring in the original string.
    /// * `end` - The ending index of the substring in the original string (exclusive).
    /// 
    /// # Returns
    /// 
    /// * `true` if the substring is equal to `other`, `false` otherwise.
    /// 
    /// # Panics
    /// 
    /// * Panics if `begin` is greater than `end`.
    /// * Panics if `end` is greater than the length of the original string.
    ///
    /// # Examples
    /// 
    /// ```
    /// use crate::utils::strings::StringExt;
    /// 
    /// let my_string = String::from("Hello, world!");
    /// let other_string = "Hello";
    /// 
    /// assert!(my_string.partial_equal(other_string, 0, 5));
    /// assert!(!my_string.partial_equal(other_string, 0, 4));
    /// assert!(!my_string.partial_equal(other_string, 7, 12));
    /// 
    /// // // This will panic because `begin` is greater than `end`
    /// /// // my_string.partial_equal(other_string, 5, 0);
    /// 
    /// // // This will panic because `end` is greater than the length of the string
    /// /// // my_string.partial_equal(other_string, 0, 20);
    /// ```
    ///
    /// # Note
    ///
    /// This method is useful for checking if a specific part of a string matches another string.
    /// It is particularly useful in scenarios where you need to validate or compare substrings
    /// without creating new string instances.
    ///
    /// # Performance
    ///
    /// This method performs a substring comparison in O(n) time complexity, where n is the length
    /// of the substring being compared. It does not allocate new strings, making it efficient
    /// for substring comparisons.
    ///
    /// # Safety
    ///
    /// This method is safe as long as the caller ensures that the `begin` and `end` indices are
    /// within the bounds of the original string. The method will panic if the indices are out of
    /// bounds, ensuring that the caller is aware of the potential issues with index handling.
    fn partial_equal(&self, other: &str, begin: usize, end: usize) -> bool;
}

impl StringExt for String {
    fn partial_equal(&self, other: &str, begin: usize, end: usize) -> bool {
        assert!(begin <= end);
        assert!(end <= self.len());
        let self_substring = &self[begin..end];
        self_substring == other
    }
}