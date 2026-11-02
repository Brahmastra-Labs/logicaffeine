use super::*;

#[test]
fn policy_default_is_fifo() {
    assert_eq!(SchedulerConfig::default().policy, SchedulePolicy::Fifo);
    assert_eq!(SchedulePolicy::default(), SchedulePolicy::Fifo);
}

#[test]
fn config_default_is_the_nice_baseline() {
    let c = SchedulerConfig::default();
    assert_eq!(c.clock, ClockMode::Logical);
    assert_eq!(c.default_channel_capacity, 32);
    assert_eq!(c.workers, 1);
}

#[test]
fn builder_overrides_apply() {
    let c = SchedulerConfig::default()
        .with_policy(SchedulePolicy::Random)
        .with_channel_capacity(4)
        .with_workers(8);
    assert_eq!(c.policy, SchedulePolicy::Random);
    assert_eq!(c.default_channel_capacity, 4);
    assert_eq!(c.workers, 8);
    assert_eq!(c.clock, ClockMode::Logical, "untouched knobs keep defaults");
}
