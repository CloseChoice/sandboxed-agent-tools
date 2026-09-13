use super::{Prefilter, load::load_window_maskless, load_window};
use crate::prefilter::{
    UnicodeChar,
    backend::{Backend, BitMaskOps},
};

impl<B: Backend> Prefilter<B> {
    #[inline(always)]
    unsafe fn match_unicode_char_prefix(
        start: usize,
        len: usize,
        haystack: &[u8],
        char_len: usize,
        chars: [u8; 4],
    ) -> B::Mask {
        debug_assert!(char_len <= 4 && char_len > 1);
        match char_len {
            2 => unsafe {
                B::eq(
                    load_window_maskless::<B>(haystack, start, len),
                    B::splat(chars[0]),
                )
            },
            3 => unsafe {
                let occ_1 = B::eq(
                    load_window_maskless::<B>(haystack, start + 1, len),
                    B::splat(chars[1]),
                );
                let occ_2 = B::eq(
                    load_window_maskless::<B>(haystack, start, len),
                    B::splat(chars[0]),
                );
                occ_1.and(occ_2)
            },
            4 => unsafe {
                let occ_1 = B::eq(
                    load_window_maskless::<B>(haystack, start + 2, len),
                    B::splat(chars[2]),
                );
                let occ_2 = B::eq(
                    load_window_maskless::<B>(haystack, start + 1, len),
                    B::splat(chars[1]),
                );
                let occ_3 = B::eq(
                    load_window_maskless::<B>(haystack, start, len),
                    B::splat(chars[0]),
                );
                occ_1.and(occ_2).and(occ_3)
            },
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    pub(super) unsafe fn unicode_char_mask(
        start: usize,
        len: usize,
        haystack: &[u8],
        needle_char: &UnicodeChar,
    ) -> B::Mask {
        let char_len = needle_char.len;
        debug_assert!(char_len <= 4 && char_len > 0);
        if start + char_len > len {
            return B::Mask::zero();
        }

        let (chunk, chunk_mask) = unsafe { load_window::<B>(haystack, start + char_len - 1, len) };
        let mut mask =
            unsafe { B::eq(chunk, B::splat(needle_char.chars[char_len - 1])) }.and(chunk_mask);
        if mask.is_zero() {
            // check the case flipped version
            mask = unsafe { B::eq(chunk, B::splat(needle_char.flipped_chars[char_len - 1])) }
                .and(chunk_mask);

            // check that the rest of the bytes in the flipped case char match
            if !mask.is_zero() && char_len > 1 {
                mask = mask.and(unsafe {
                    Self::match_unicode_char_prefix(
                        start,
                        len,
                        haystack,
                        char_len,
                        needle_char.flipped_chars,
                    )
                });
            }
        } else if char_len > 1 {
            mask = mask.and(unsafe {
                Self::match_unicode_char_prefix(start, len, haystack, char_len, needle_char.chars)
            });
        }
        mask
    }

    // Local patch: the original SIMD loop reused a byte window when the next
    // needle scalar had a different UTF-8 width (e.g. "café"), rejecting valid
    // matches. Scan scalar boundaries for the zero-typo Unicode prefilter.
    // Preserve the earliest start and latest matching final scalar so the
    // scoring stage can choose the best alignment inside that window.
    #[inline(always)]
    pub unsafe fn match_haystack_unicode(&self, haystack: &[u8]) -> (bool, usize, usize) {
        let Ok(text) = std::str::from_utf8(haystack) else {
            return (false, 0, haystack.len());
        };
        let mut needle_index = 0;
        let mut match_start = 0;
        for (offset, ch) in text.char_indices() {
            let needle = &self.needle_unicode[needle_index];
            let bytes = &haystack[offset..offset + ch.len_utf8()];
            if bytes != &needle.chars[..needle.len]
                && bytes != &needle.flipped_chars[..needle.len]
            {
                continue;
            }
            if needle_index == 0 {
                match_start = offset;
            }
            needle_index += 1;
            if needle_index == self.needle_unicode.len() {
                let last = needle;
                let end = text.char_indices().rev().find_map(|(pos, c)| {
                    let bytes = &haystack[pos..pos + c.len_utf8()];
                    (bytes == &last.chars[..last.len]
                        || bytes == &last.flipped_chars[..last.len])
                        .then_some(pos + c.len_utf8())
                }).unwrap();
                return (true, match_start, end);
            }
        }
        (false, match_start, haystack.len())
    }

    #[inline(always)]
    #[allow(dead_code)] // Retained upstream helper; local zero-typo patch does not use it.
    unsafe fn find_last_unicode_char_pos(needle_char: &UnicodeChar, haystack: &[u8]) -> usize {
        let len = haystack.len();
        let char_len = needle_char.len;
        debug_assert!(char_len <= 4 && char_len > 0);
        debug_assert!(len >= char_len);

        let last_bytes = unsafe {
            (
                B::splat(needle_char.chars[char_len - 1]),
                B::splat(needle_char.flipped_chars[char_len - 1]),
            )
        };
        let mut start = len.saturating_sub(B::LANES + char_len - 1);
        loop {
            let (chunk, chunk_mask) =
                unsafe { load_window::<B>(haystack, start + char_len - 1, len) };

            // we check both the original and flipped case bytes simultaneously since either
            // could have matched. this can result in a false positive, but that's fine for
            // this stage since this just controls bounds, and bounds being too large hurt
            // performance, not correctness.
            let mut mask = unsafe { B::eq(chunk, last_bytes.0).or(B::eq(chunk, last_bytes.1)) }
                .and(chunk_mask);

            if !mask.is_zero() && char_len > 1 {
                mask = mask.and(unsafe {
                    Self::match_unicode_char_prefix(
                        start,
                        len,
                        haystack,
                        char_len,
                        needle_char.chars,
                    )
                    .or(Self::match_unicode_char_prefix(
                        start,
                        len,
                        haystack,
                        char_len,
                        needle_char.flipped_chars,
                    ))
                });
            }

            if !mask.is_zero() {
                return start + B::LANES - mask.leading_zeros() + char_len - 1;
            }

            if start == 0 {
                break;
            }
            start = start.saturating_sub(B::LANES);
        }

        len
    }
}
