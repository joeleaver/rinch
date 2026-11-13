//! #1788: the shared device's slot, on slots of the tests' own (the process's
//! is shared by every GPU test). The loss itself is real: `Device::destroy`
//! loses a device, and wgpu calls its device-lost callback at the next poll.
//! The GPU tests are `#[ignore]`d: they need an adapter (lavapipe will do,
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_device`).
use std::cell::RefCell;
use std::rc::Rc;

use super::*;

fn leaked_slot() -> &'static GpuSlot {
    Box::leak(Box::new(GpuSlot::new()))
}

fn new_device() -> (Instance, GpuHandle) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("a GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("a device");
    let handle = GpuHandle::new(Arc::new(device), Arc::new(queue), Arc::new(adapter));
    (instance, handle)
}

/// Lose `device` as a driver reset would, and let wgpu report it.
fn lose(device: &Device) {
    device.destroy();
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

#[test]
fn a_loss_is_due_once_and_a_stale_one_is_not() {
    let slot = leaked_slot();
    slot.lost(7, DeviceLostReason::Unknown, "not this slot's device");
    assert!(!slot.take_due(), "a device of another generation");
    slot.lost(0, DeviceLostReason::Unknown, "driver reset");
    assert!(slot.take_due(), "the current device's loss is due");
    assert!(!slot.take_due(), "and due once");
}

#[test]
fn listeners_hear_each_replacement_and_may_add_listeners() {
    let heard: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let h = heard.clone();
    on_gpu_device_replaced(move |handle| {
        h.borrow_mut().push(if handle.is_some() {
            "first: some"
        } else {
            "first: none"
        });
        let again = h.clone();
        on_gpu_device_replaced(move |_| again.borrow_mut().push("added"));
    });
    announce(None);
    assert_eq!(*heard.borrow(), ["first: none"]);
    announce(None);
    assert_eq!(*heard.borrow(), ["first: none", "first: none", "added"]);
    LISTENERS.with(|l| l.borrow_mut().clear());
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_loss_is_detected_and_the_device_replaced_1788() {
    let slot = leaked_slot();
    let (instance, first) = new_device();
    let device = first.device.clone();
    let generation = slot.publish(instance, first).expect("the first device");
    slot.watch(&device, generation);
    assert_eq!(generation, 1);
    assert!(
        slot.take_announcement().is_none(),
        "the first device is no news"
    );

    lose(&device);
    assert!(slot.take_due(), "the loss reached the slot");

    slot.retire();
    assert!(slot.handle().is_none(), "the lost device is not handed out");
    assert!(
        matches!(slot.take_announcement(), Some(None)),
        "between the two, the app hears of no device"
    );
    let (instance, second) = new_device();
    let replacement = second.device.clone();
    let generation = slot.publish(instance, second).expect("the next device");
    assert_eq!(generation, 3, "a retirement and a publish later");
    let Some(Some(announced)) = slot.take_announcement() else {
        panic!("the replacement is announced");
    };
    assert!(Arc::ptr_eq(&announced.device, &replacement));
    assert_eq!(announced.generation(), slot.generation());
    assert!(Arc::ptr_eq(&slot.handle().unwrap().device, &replacement));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_loss_of_a_retired_device_is_ignored_1788() {
    let slot = leaked_slot();
    let (instance, first) = new_device();
    let device = first.device.clone();
    let generation = slot.publish(instance, first).expect("the first device");
    slot.watch(&device, generation);
    slot.retire();
    lose(&device);
    assert!(!slot.take_due(), "a device already replaced reports late");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_an_external_replacement_is_due_and_presented_with_1788() {
    let slot = leaked_slot();
    let (instance, handle) = new_device();
    let configured = ExternalGpu {
        instance,
        adapter: handle.adapter,
        device: handle.device,
        queue: handle.queue,
    };
    assert!(Arc::ptr_eq(
        &slot.external_source(&configured).device,
        &configured.device
    ));
    let (instance, handle) = new_device();
    let replacement = handle.device.clone();
    slot.replace_external(ExternalGpu {
        instance,
        adapter: handle.adapter,
        device: handle.device,
        queue: handle.queue,
    });
    assert!(slot.take_due(), "the app's replacement is due");
    assert!(Arc::ptr_eq(
        &slot.external_source(&configured).device,
        &replacement
    ));
}

/// Map a buffer made on `device`: a validation error once the device is
/// gone (the buffer is invalid), which wgpu's default handler panics on.
fn provoke_error(device: &Device) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("made on a lost device"),
        size: 256,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
}

/// A frame drawn on the device between its loss and its replacement makes
/// errors; they are dropped, not a panic. `destroy` without a poll is the
/// hard case: the device reports its loss only at a poll, which the error
/// handler gives it.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_errors_on_a_lost_device_do_not_panic_1788() {
    let slot = leaked_slot();
    let (instance, first) = new_device();
    let device = first.device.clone();
    let generation = slot.publish(instance, first).expect("the first device");
    slot.watch(&device, generation);
    device.destroy();
    provoke_error(&device);
    assert!(
        slot.take_due(),
        "the handler's poll let the loss be reported"
    );
    provoke_error(&device);
}
