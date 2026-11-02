use super::*;
use crate::codegen_sva::fp_hypotheses::family;
use crate::codegen_sva::fp_oracle::Class;

/// The card's own `pr.log` after configuring fp-oracle5's persona.
const PR_CLEAN: &str = "PR_STS  0x00000000000001ff: CREDIT 511 STS 0 CTRLR_STS 0 HOST_STS 0\n\
PR_ERR  0x0000000000000000: OPERATION 0 CRC 0 INCOMPATIBLE 0 PROTOCOL 0 FIFO_OVERFLOW 0\n";

#[test]
fn the_configured_image_is_the_first_word_of_the_runs_sha256sum_line() {
    let sha = "984b31665a42f04d0588e3bae4ac3d00ec57b7a10604bb86e6bf961d8ab04657";
    assert_eq!(configured(&format!("{sha}  /srv/card/fp-oracle5/logos_d5005.gbs\n")), Some(sha));
    assert_eq!(configured(&format!("{sha}\n")), Some(sha));
    assert_eq!(configured(&sha[1..]), None, "63 digits");
    assert_eq!(configured(&format!("{}  x.gbs", sha.to_uppercase())), None, "sha256sum writes lower case");
    assert_eq!(configured(&format!("{}g  x.gbs", &sha[1..])), None, "a digit that is no hex");
    assert_eq!(configured(""), None);
}

#[test]
fn reconfiguration_holds_only_when_the_error_register_and_every_field_of_it_are_zero() {
    assert_eq!(reconfiguration(PR_CLEAN), Ok(()));
    let crc = PR_CLEAN.replace("CRC 0", "CRC 1");
    assert!(reconfiguration(&crc).unwrap_err().contains("CRC 1"), "a field set is named");
    let raw = PR_CLEAN.replace("0x0000000000000000", "0x0000000000000004");
    assert!(reconfiguration(&raw).is_err(), "the register nonzero though its fields read zero");
    assert!(reconfiguration(PR_CLEAN.lines().next().unwrap()).is_err(), "no error register");
    let unread = PR_CLEAN.replace("FIFO_OVERFLOW 0", "FIFO_OVERFLOW");
    assert!(reconfiguration(&unread).is_err(), "a field with no value");
    let twice = format!("{PR_CLEAN}{}", PR_CLEAN.lines().nth(1).unwrap().replace("PROTOCOL 0", "PROTOCOL 1"));
    assert!(reconfiguration(&twice).is_err(), "every error register the run left");
}

#[test]
fn the_user_clock_is_the_last_high_clock_the_run_reported() {
    let log = " Freq High             : 100\n Freq Low              : 50\n\nAFU Resource found.\n\nApproximate frequency:\n\
High clock = 100.0 MHz\nLow clock  =  50.0 MHz\n \n\nApproximate frequency:\nHigh clock = 100.0 MHz\nLow clock  =  50.0 MHz\n";
    assert_eq!(user_clock_mhz(log), Some(100.0));
    let moved = format!("{log}Approximate frequency:\nHigh clock = 312.5 MHz\n");
    assert_eq!(user_clock_mhz(&moved), Some(312.5), "the clock the run left set");
    assert_eq!(user_clock_mhz(" Freq High             : 100\n"), None, "asked for, never reported");
    assert_eq!(user_clock_mhz("High clock = fast MHz\n"), None);
}

#[test]
fn a_time_is_a_utc_second_in_the_one_shape_the_run_writes() {
    assert!(time("2026-10-25T00:40:44Z"));
    assert!(!time("2026-10-25 00:40:44Z"));
    assert!(!time("2026-10-25T00:40:44"));
    assert!(!time("2026-10-25T00:40:44.5Z"));
    assert!(!time("2026-1a-25T00:40:44Z"));
    assert!(!time(""));
}

fn class(name: &str) -> Class {
    let semantics = family().into_iter().find(|s| s.name() == name).unwrap();
    Class { members: vec![name.to_string()], semantics }
}

#[test]
fn the_nearest_classes_are_the_fewest_differing_first_in_the_familys_order_on_a_tie() {
    let names: Vec<String> = family().iter().take(5).map(|s| s.name()).collect();
    let classes: Vec<Class> = names.iter().map(|n| class(n)).collect();
    let near = nearest(&classes, &[7, 0, 3, 3, 9]);
    assert_eq!(
        near,
        vec![
            Near { class: names[1].clone(), differing: 0 },
            Near { class: names[2].clone(), differing: 3 },
            Near { class: names[3].clone(), differing: 3 },
        ]
    );
    assert_eq!(nearest(&classes[..2], &[5, 1]).len(), 2, "no more than the family has");
}

#[test]
fn a_kept_sha256sum_line_names_its_image_by_its_file_name_alone() {
    let sha = "984b31665a42f04d0588e3bae4ac3d00ec57b7a10604bb86e6bf961d8ab04657";
    assert_eq!(kept("gbs.sha256", &format!("{sha}  /srv/card/fp-oracle5/logos_d5005.gbs\n")), format!("{sha}  logos_d5005.gbs\n"));
    assert_eq!(kept("gbs.sha256", &format!("{sha}  logos_d5005.gbs\n")), format!("{sha}  logos_d5005.gbs\n"), "a bare name kept as it is");
    assert_eq!(kept("gbs.sha256", &format!("{sha}  /srv/x.gbs")), format!("{sha}  x.gbs"), "no newline added");
    let pr = "PR_ERR  0x0000000000000000: OPERATION 0 CRC 0\n";
    assert_eq!(kept("pr.log", pr), pr, "every other file as the run wrote it");
    assert_eq!(configured(&kept("gbs.sha256", &format!("{sha}  /srv/card/logos_d5005.gbs\n"))), Some(sha), "the image read the same");
}

