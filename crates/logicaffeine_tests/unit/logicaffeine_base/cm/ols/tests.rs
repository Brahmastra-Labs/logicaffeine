use super::*;

/// Run the predictor over an image, returning the mean absolute residual over its last `tail` samples.
fn residual_after(image: &[u16], width: usize, tail: usize) -> f64 {
    let mut ols = Ols::new();
    let mut samples = Vec::with_capacity(image.len());
    let mut total = 0.0;
    for (k, &s) in image.iter().enumerate() {
        let f = Ols::features(&samples, k, width);
        let p = ols.predict(&f);
        if k >= image.len() - tail {
            total += (f64::from(s) - f64::from(p)).abs();
        }
        samples.push(s);
        ols.update(&f, s);
    }
    total / tail as f64
}

#[test]
fn the_neighbours_are_read_causally_and_outside_the_image_as_zero() {
    let width = 4;
    let samples: Vec<u16> = (1..=12).collect();
    // Sample 9 is row 2, column 1: W = 9, N = 6, NW = 5, NE = 7, WW = outside, NN = 2, NNE = 3, NWW = outside, NEE = 8,
    // NNW = 1, and the constant.
    assert_eq!(Ols::features(&samples, 9, width), [9.0, 6.0, 5.0, 7.0, 0.0, 2.0, 3.0, 0.0, 8.0, 1.0, 1.0]);
    assert_eq!(Ols::features(&samples, 0, width)[..10], [0.0; 10], "the first sample has no neighbours");
    let f = Ols::features(&samples[..5], 5, width);
    assert_eq!(f[0], 5.0, "W is the sample just before");
}

#[test]
fn a_linear_image_is_learnt_exactly() {
    // x = 3·col + 5·row + 100: every sample is W + N − NW, a combination the predictor can find.
    let (width, height) = (64, 64);
    let image: Vec<u16> = (0..width * height).map(|k| (100 + 3 * (k % width) + 5 * (k / width)) as u16).collect();
    let tail = residual_after(&image, width, 1000);
    assert!(tail < 0.05, "mean residual {tail:.3} once learnt");
}

#[test]
fn it_beats_the_mean_of_west_and_north_on_a_textured_image_and_agrees_with_itself() {
    let (width, height) = (96, 96);
    let mut s = 7u64;
    let image: Vec<u16> = (0..width * height)
        .map(|k| {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let (x, y) = ((k % width) as f64, (k / width) as f64);
            (2000.0 + 400.0 * (x / 9.0).sin() * (y / 13.0).cos() + 3.0 * x - 2.0 * y + (s >> 61) as f64) as u16
        })
        .collect();
    let mean = {
        let mut total = 0.0;
        for k in image.len() - 2000..image.len() {
            let (w, n) = (f64::from(image[k - 1]), f64::from(image[k - width]));
            total += (f64::from(image[k]) - ((w + n) / 2.0).round()).abs();
        }
        total / 2000.0
    };
    let ols = residual_after(&image, width, 2000);
    assert!(ols < mean * 0.5, "least squares {ols:.2} against (W + N) / 2 {mean:.2}");
    assert_eq!(residual_after(&image, width, 2000), ols, "the same samples give the same predictions");
}

#[test]
fn a_flat_image_never_makes_it_singular() {
    let image = vec![1234u16; 50 * 50];
    assert_eq!(residual_after(&image, 50, 500), 0.0);
}
