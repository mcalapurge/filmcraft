//! Word timing from cross-attention: token-to-audio alignment by dynamic time warping over the
//! averaged, normalised attention of the alignment heads (see the module docs of `whisper`).

use candle_core::{Result, Tensor};

use super::model::Model;

/// Start/end (in mel frames from the window start, 10 ms) of every token of `text`.
pub fn align(m: &mut Model, xa: &Tensor, prompt: &[u32], text: &[u32], eot: u32, heads: &[(usize, usize)], seg_frames: usize) -> Result<Vec<(usize, usize)>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if prompt.is_empty() || heads.is_empty() || seg_frames < 2 {
        candle_core::bail!("word alignment requires a prompt, alignment heads and audio frames");
    }
    let mut tokens = prompt.to_vec();
    tokens.extend_from_slice(text);
    tokens.push(eot);
    m.decoder.reset();
    let (_, qk) = m.decoder.forward(&tokens, xa, false, Some(heads))?;
    m.decoder.reset();
    let Some(qk) = qk else { candle_core::bail!("word-alignment attention is unavailable") };
    let (h, t, f_all) = qk.dims3()?;
    if h == 0 || f_all == 0 || t != tokens.len() {
        candle_core::bail!("invalid word-alignment attention dimensions");
    }
    let f = (seg_frames / 2).min(f_all);
    let qk = qk.narrow(2, 0, f)?.contiguous()?;
    let w = candle_nn::ops::softmax_last_dim(&qk)?;
    let w: Vec<Vec<Vec<f32>>> = w.to_vec3()?;
    // The decoder row of input token i attends to the audio of the token it predicts (i + 1),
    // so token k's audio is where the path runs through the row before it: the final prompt
    // token's row for the first text token, and so on. The trailing EOT row takes the audio
    // after the last word, so the last word ends where speech does.
    let rows = text.len() + 2;
    let first = prompt.len() - 1;
    if rows.checked_add(1).and_then(|r| f.checked_add(1).and_then(|c| r.checked_mul(c))).is_none_or(|cells| cells > 4_000_000) {
        candle_core::bail!("word-alignment matrix is too large");
    }
    let mut matrix = vec![0f32; rows * f];
    for head in w.iter().take(h) {
        // standardise over tokens for every frame
        let mut z = vec![0f32; t * f];
        for j in 0..f {
            let mean = (0..t).map(|i| head[i][j]).sum::<f32>() / t as f32;
            let var = (0..t).map(|i| (head[i][j] - mean).powi(2)).sum::<f32>() / t as f32;
            let sd = var.sqrt().max(1e-8);
            for i in 0..t {
                z[i * f + j] = (head[i][j] - mean) / sd;
            }
        }
        for r in 0..rows {
            let row = median_filter(&z[(first + r) * f..(first + r + 1) * f], 7);
            for j in 0..f {
                matrix[r * f + j] += row[j] / h as f32;
            }
        }
    }
    let neg: Vec<f32> = matrix.iter().map(|v| -v).collect();
    let out = token_bounds(&neg, text.len(), f)?;
    Ok(out)
}

/// Costs include a leading prompt row, one row per text token and a trailing EOT row. Token k
/// spans the frames where the path enters row k (the row that predicts it) up to row k + 1.
fn token_bounds(cost: &[f32], tokens: usize, frames: usize) -> Result<Vec<(usize, usize)>> {
    let Some(rows) = tokens.checked_add(2) else { candle_core::bail!("too many alignment tokens") };
    let path = dtw(cost, rows, frames);
    if path.is_empty() {
        candle_core::bail!("invalid or oversized word-alignment matrix");
    }
    let mut jump = vec![frames; rows];
    for (row, frame) in path {
        if let Some(boundary) = jump.get_mut(row) {
            *boundary = (*boundary).min(frame);
        }
    }
    Ok(jump.windows(2).take(tokens).map(|pair| (pair[0] * 2, pair[1].max(pair[0]) * 2)).collect())
}

/// Median filter with reflect padding.
pub fn median_filter(x: &[f32], width: usize) -> Vec<f32> {
    let n = x.len();
    if n == 0 || width <= 1 {
        return x.to_vec();
    }
    let half = width / 2;
    let at = |i: isize| -> f32 {
        if n == 1 {
            return x[0];
        }
        let p = 2 * (n as isize - 1);
        let mut j = i.rem_euclid(p);
        if j >= n as isize {
            j = p - j;
        }
        x[j as usize]
    };
    let mut buf = vec![0f32; width];
    (0..n)
        .map(|i| {
            for (k, b) in buf.iter_mut().enumerate() {
                *b = at(i as isize + k as isize - half as isize);
            }
            buf.sort_by(f32::total_cmp);
            buf[half]
        })
        .collect()
}

/// Dynamic time warping through `cost` (`n × m`, row-major), monotonic steps (diagonal, down,
/// right). Returns the path from (0, 0) to (n − 1, m − 1).
pub fn dtw(cost: &[f32], n: usize, m: usize) -> Vec<(usize, usize)> {
    // Attention windows are bounded (normally <= 226 × 1500). Reject malformed
    // dimensions before allocating or indexing, including arithmetic overflow.
    let cells = n.checked_add(1).and_then(|r| m.checked_add(1).and_then(|c| r.checked_mul(c)));
    if n == 0 || m == 0 || n.checked_mul(m) != Some(cost.len()) || cells.is_none_or(|c| c > 4_000_000) || cost.iter().any(|c| !c.is_finite()) {
        return Vec::new();
    }
    let inf = f32::INFINITY;
    let mut acc = vec![inf; (n + 1) * (m + 1)];
    let mut trace = vec![0u8; (n + 1) * (m + 1)];
    acc[0] = 0.0;
    let idx = |i: usize, j: usize| i * (m + 1) + j;
    for j in 1..=m {
        for i in 1..=n {
            let c0 = acc[idx(i - 1, j - 1)];
            let c1 = acc[idx(i - 1, j)];
            let c2 = acc[idx(i, j - 1)];
            let (c, t) = if c0 < c1 && c0 < c2 {
                (c0, 0)
            } else if c1 < c0 && c1 < c2 {
                (c1, 1)
            } else {
                (c2, 2)
            };
            acc[idx(i, j)] = cost[(i - 1) * m + (j - 1)] + c;
            trace[idx(i, j)] = t;
        }
    }
    for j in 0..=m {
        trace[idx(0, j)] = 2;
    }
    for i in 0..=n {
        trace[idx(i, 0)] = 1;
    }
    let (mut i, mut j) = (n, m);
    let mut path = Vec::new();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 {
            path.push((i - 1, j - 1));
        }
        match trace[idx(i, j)] {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_takes_the_frames_of_the_row_that_predicts_it() {
        // Prompt row (predicting "It") covers 5–5.2s, the "It" row (predicting EOT) covers the
        // pause after it, EOT the rest. Units are 20ms. Leading audio before 5s stays with the
        // prompt row's entry (frame 0); the energy trim in `vad` removes it.
        let mut cost = vec![10.0; 3 * 300];
        cost[250..260].fill(-10.0);
        cost[300 + 260..300 + 280].fill(-10.0);
        cost[600 + 280..].fill(-10.0);
        assert_eq!(token_bounds(&cost, 1, 300).unwrap(), vec![(0, 520)]);
    }

    #[test]
    fn consecutive_tokens_are_not_shifted_by_one() {
        // regression: tokens took the frames of the row after the one predicting them, so every
        // word landed one token late
        let mut cost = vec![10.0; 4 * 10];
        cost[..4].fill(-10.0); // prompt row: first token at 0–80ms
        cost[14..17].fill(-10.0); // first token's row: second token at 80–140ms
        cost[27..30].fill(-10.0); // second token's row: the pause, 140–200ms
        cost[30..].fill(-10.0); // EOT
        assert_eq!(token_bounds(&cost, 2, 10).unwrap(), vec![(0, 8), (8, 14)]);
    }

    #[test]
    fn alignment_rejects_hostile_dimensions_without_panicking() {
        for (cost, n, m) in [(vec![], 0, 0), (vec![0.0], usize::MAX, 1), (vec![0.0], 2, 3), (vec![f32::NAN], 1, 1)] {
            assert!(dtw(&cost, n, m).is_empty());
        }
        assert!(token_bounds(&[], usize::MAX, 1).is_err());
    }

    #[test]
    fn median_filter_removes_spikes() {
        assert_eq!(median_filter(&[0.0, 0.0, 9.0, 0.0, 0.0], 3), vec![0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(median_filter(&[1.0, 2.0, 3.0], 1), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn dtw_follows_the_cheap_diagonal_band() {
        // 3 rows × 6 columns, cheap cells at row 0: cols 0-1, row 1: cols 2-3, row 2: cols 4-5
        let mut c = vec![1.0f32; 18];
        for (r, cols) in [(0, [0, 1]), (1, [2, 3]), (2, [4, 5])] {
            for k in cols {
                c[r * 6 + k] = 0.0;
            }
        }
        let p = dtw(&c, 3, 6);
        assert_eq!(p.first(), Some(&(0, 0)));
        assert_eq!(p.last(), Some(&(2, 5)));
        let first_col = |row: usize| p.iter().find(|x| x.0 == row).unwrap().1;
        assert_eq!((first_col(0), first_col(1), first_col(2)), (0, 2, 4));
    }
}
