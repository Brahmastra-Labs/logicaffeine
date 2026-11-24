use super::*;

#[test]
fn a_cpu_list_counts_every_cpu_it_names() {
    assert_eq!(cpus_in("0-15"), Some(16));
    assert_eq!(cpus_in("0-15,32,34-35"), Some(19));
    assert_eq!(cpus_in("7"), Some(1));
    assert_eq!(cpus_in("0,2,4,6"), Some(4));
}

#[test]
fn text_that_is_not_a_cpu_list_counts_none() {
    for text in ["", "a-b", "3-1", "0-", "-4", "0-15,", "0,,1"] {
        assert_eq!(cpus_in(text), None, "{text:?}");
    }
}

#[test]
fn the_host_runs_at_least_one_thread() {
    let (threads, hybrid) = host_threads();
    assert!(threads >= 1);
    assert!(!hybrid || threads <= std::thread::available_parallelism().map_or(usize::MAX, |n| n.get()), "a hybrid host's performance cores are among its threads");
}
