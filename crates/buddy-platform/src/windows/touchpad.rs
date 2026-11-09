use crate::touchpad_scroll::{Contact, TouchFrame, TouchpadScroll};
use buddy_project::WheelDirection;
use std::{
    collections::BTreeMap,
    mem::size_of,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Devices::HumanInterfaceDevice::*,
    Foundation::HANDLE,
    UI::{
        Input::{GetRawInputDeviceInfoW, RIDI_PREPARSEDDATA},
        WindowsAndMessaging::SystemParametersInfoW,
    },
};

pub(super) const DIGITIZER_PAGE: u16 = 0x0d;
pub(super) const TOUCHPAD_USAGE: u16 = 0x05;
const DESKTOP_PAGE: u16 = 0x01;
const BUTTON_PAGE: u16 = 0x09;
const X: u16 = 0x30;
const Y: u16 = 0x31;
const TIP: u16 = 0x42;
const CONFIDENCE: u16 = 0x47;
const CONTACT_ID: u16 = 0x51;
const CONTACT_COUNT: u16 = 0x54;
const SCAN_TIME: u16 = 0x56;

#[derive(Default)]
pub(super) struct Touchpads {
    devices: BTreeMap<usize, Option<Device>>,
    settings: Option<(Instant, Option<ScrollSettings>)>,
}

impl Touchpads {
    pub(super) fn reset(&mut self) {
        for device in self.devices.values_mut().flatten() {
            device.reset();
        }
        self.settings = None;
    }

    pub(super) fn remove(&mut self, device: usize) {
        self.devices.remove(&device);
    }

    pub(super) fn report(
        &mut self,
        handle: HANDLE,
        report: &[u8],
        at: Instant,
    ) -> Option<WheelDirection> {
        if handle.is_null() {
            return None;
        }
        let device = self
            .devices
            .entry(handle as usize)
            .or_insert_with(|| unsafe { Device::open(handle) })
            .as_mut()?;
        if self
            .settings
            .is_none_or(|(last, _)| at.saturating_duration_since(last) >= Duration::from_secs(1))
        {
            let settings = read_settings();
            if self
                .settings
                .is_some_and(|(_, previous)| previous != settings)
            {
                device.reset();
            }
            self.settings = Some((at, settings));
        }
        let settings = self.settings.and_then(|(_, value)| value)?;
        if !settings.enabled {
            device.reset();
            return None;
        }
        let frame = device.frame(report)?;
        device.scroll.update(frame, at, settings.reversed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ScrollSettings {
    enabled: bool,
    reversed: bool,
}

// TOUCHPAD_PARAMETERS_V1 and SPI_GETTOUCHPADPARAMETERS are available on the
// supported Windows 11 versions but are missing from our windows-sys bindings.
#[repr(C)]
struct TouchpadParametersV1 {
    version_number: u32,
    max_supported_contacts: u32,
    legacy_touchpad_features: u32,
    device_flags: u32,
    settings_flags: u32,
    sensitivity_level: u32,
    cursor_speed: u32,
    feedback_intensity: u32,
    click_force_sensitivity: u32,
    right_click_zone_width: u32,
    right_click_zone_height: u32,
}

fn read_settings() -> Option<ScrollSettings> {
    const SPI_GETTOUCHPADPARAMETERS: u32 = 0x00ae;
    const TOUCHPAD_ACTIVE: u32 = 1 << 4;
    const PAN_ENABLED: u32 = 1 << 7;
    const SCROLL_REVERSED: u32 = 1 << 9;
    let mut params: TouchpadParametersV1 = unsafe { std::mem::zeroed() };
    params.version_number = 1;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETTOUCHPADPARAMETERS,
            size_of::<TouchpadParametersV1>() as u32,
            (&mut params as *mut TouchpadParametersV1).cast(),
            0,
        )
    };
    (ok != 0).then_some(ScrollSettings {
        enabled: params.device_flags & TOUCHPAD_ACTIVE != 0
            && params.settings_flags & PAN_ENABLED != 0,
        reversed: params.settings_flags & SCROLL_REVERSED != 0,
    })
}

struct Device {
    descriptor: Vec<usize>,
    input_bytes: usize,
    slots: BTreeMap<u8, Vec<ContactSlot>>,
    scan: Option<u32>,
    pending: Option<Contact>,
    scroll: TouchpadScroll,
}

struct ContactSlot {
    link: u16,
    x: AxisRange,
    y: AxisRange,
}

struct AxisRange {
    min: f64,
    max: f64,
    bit_size: u16,
    mm_per_unit: f64,
}

impl AxisRange {
    fn from_cap(cap: &HIDP_VALUE_CAPS) -> Option<Self> {
        let min = f64::from(cap.LogicalMin);
        let max = f64::from(cap.LogicalMax);
        if min >= max || !(1..=32).contains(&cap.BitSize) {
            return None;
        }
        let exponent = ((cap.UnitsExp as i8) << 4) >> 4;
        let unit_mm = match cap.Units {
            0x11 => 10.0, // SI linear centimeter
            0x13 => 25.4, // English linear inch
            _ => return None,
        };
        let span_mm = (f64::from(cap.PhysicalMax) - f64::from(cap.PhysicalMin))
            * unit_mm
            * 10_f64.powi(i32::from(exponent));
        (span_mm > 0.0).then_some(Self {
            min,
            max,
            bit_size: cap.BitSize,
            mm_per_unit: span_mm / (max - min),
        })
    }

    fn to_mm(&self, raw: u32) -> Option<f64> {
        let value = if self.min < 0.0 {
            let shift = 32 - self.bit_size;
            f64::from(((raw << shift) as i32) >> shift)
        } else {
            f64::from(raw)
        };
        (self.min..=self.max)
            .contains(&value)
            .then_some((value - self.min) * self.mm_per_unit)
    }
}

impl Device {
    unsafe fn open(handle: HANDLE) -> Option<Self> {
        let mut bytes = 0;
        if GetRawInputDeviceInfoW(handle, RIDI_PREPARSEDDATA, ptr::null_mut(), &mut bytes) != 0
            || !(1..=65536).contains(&bytes)
        {
            return None;
        }
        let mut descriptor = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetRawInputDeviceInfoW(
            handle,
            RIDI_PREPARSEDDATA,
            descriptor.as_mut_ptr().cast(),
            &mut bytes,
        ) != bytes
        {
            return None;
        }
        let data = descriptor.as_ptr() as PHIDP_PREPARSED_DATA;
        let mut caps = HIDP_CAPS::default();
        if HidP_GetCaps(data, &mut caps) != HIDP_STATUS_SUCCESS
            || (caps.UsagePage, caps.Usage) != (DIGITIZER_PAGE, TOUCHPAD_USAGE)
            || !(1..=1024).contains(&caps.NumberInputValueCaps)
        {
            return None;
        }
        let mut count = caps.NumberInputValueCaps;
        let mut values = vec![HIDP_VALUE_CAPS::default(); usize::from(count)];
        if HidP_GetValueCaps(HidP_Input, values.as_mut_ptr(), &mut count, data)
            != HIDP_STATUS_SUCCESS
        {
            return None;
        }
        let mut axes = BTreeMap::<_, (Option<AxisRange>, Option<AxisRange>)>::new();
        for cap in &values[..usize::from(count)] {
            if cap.UsagePage != DESKTOP_PAGE
                || cap.LinkCollection == 0
                || !cap.IsAbsolute
                || cap.IsAlias
            {
                continue;
            }
            let (first, last) = if cap.IsRange {
                let range = cap.Anonymous.Range;
                (range.UsageMin, range.UsageMax)
            } else {
                let usage = cap.Anonymous.NotRange.Usage;
                (usage, usage)
            };
            let (x, y) = axes.entry((cap.ReportID, cap.LinkCollection)).or_default();
            if (first..=last).contains(&X) {
                *x = AxisRange::from_cap(cap);
            }
            if (first..=last).contains(&Y) {
                *y = AxisRange::from_cap(cap);
            }
        }
        let mut slots = BTreeMap::<_, Vec<_>>::new();
        for ((report_id, link), (x, y)) in axes {
            if let (Some(x), Some(y)) = (x, y) {
                slots
                    .entry(report_id)
                    .or_default()
                    .push(ContactSlot { link, x, y });
            }
        }
        (!slots.is_empty()).then_some(Self {
            descriptor,
            input_bytes: usize::from(caps.InputReportByteLength),
            slots,
            scan: None,
            pending: None,
            scroll: TouchpadScroll::default(),
        })
    }

    fn reset(&mut self) {
        self.scan = None;
        self.pending = None;
        self.scroll = TouchpadScroll::default();
    }

    fn value(&self, report: &[u8], page: u16, link: u16, usage: u16) -> Option<u32> {
        let mut value = 0;
        let status = unsafe {
            HidP_GetUsageValue(
                HidP_Input,
                page,
                link,
                usage,
                &mut value,
                self.descriptor.as_ptr() as PHIDP_PREPARSED_DATA,
                report.as_ptr(),
                report.len() as u32,
            )
        };
        (status == HIDP_STATUS_SUCCESS).then_some(value)
    }

    fn buttons(&self, report: &[u8], page: u16, link: u16, usages: &mut [u16]) -> usize {
        let mut count = usages.len() as u32;
        let status = unsafe {
            HidP_GetUsages(
                HidP_Input,
                page,
                link,
                usages.as_mut_ptr(),
                &mut count,
                self.descriptor.as_ptr() as PHIDP_PREPARSED_DATA,
                report.as_ptr().cast_mut(), // HidP_GetUsages takes an input-only report.
                report.len() as u32,
            )
        };
        if status == HIDP_STATUS_SUCCESS {
            (count as usize).min(usages.len())
        } else {
            0
        }
    }

    fn contact(&self, report: &[u8], slot: &ContactSlot) -> Option<Contact> {
        let mut buttons = [0; 16];
        let count = self.buttons(report, DIGITIZER_PAGE, slot.link, &mut buttons);
        if !buttons[..count].contains(&TIP) || !buttons[..count].contains(&CONFIDENCE) {
            return None;
        }
        Some(Contact {
            id: self.value(report, DIGITIZER_PAGE, slot.link, CONTACT_ID)?,
            x_mm: slot
                .x
                .to_mm(self.value(report, DESKTOP_PAGE, slot.link, X)?)?,
            y_mm: slot
                .y
                .to_mm(self.value(report, DESKTOP_PAGE, slot.link, Y)?)?,
        })
    }

    fn frame(&mut self, report: &[u8]) -> Option<TouchFrame> {
        if report.len() != self.input_bytes {
            return None;
        }
        let slots = self.slots.get(report.first()?)?;
        let count = self.value(report, DIGITIZER_PAGE, 0, CONTACT_COUNT)?;
        let scan = self.value(report, DIGITIZER_PAGE, 0, SCAN_TIME)?;
        // In hybrid reports the first packet contains the count; subsequent
        // packets from the same scan contain the remaining contacts and zero.
        let first = if count != 0 || self.scan != Some(scan) {
            self.scan = Some(scan);
            self.pending = None;
            match count {
                0 | 1 => return Some(TouchFrame::Released),
                2 => None,
                _ => return Some(TouchFrame::OtherGesture),
            }
        } else {
            Some(self.pending.take()?)
        };
        if self.buttons(report, BUTTON_PAGE, 0, &mut [0; 16]) != 0 {
            return Some(TouchFrame::OtherGesture);
        }
        let Some(contact) = self.contact(report, &slots[0]) else {
            return Some(TouchFrame::Released);
        };
        if let Some(first) = first {
            return Some(TouchFrame::Pair([first, contact]));
        }
        if let Some(slot) = slots.get(1) {
            return Some(
                self.contact(report, slot)
                    .map_or(TouchFrame::Released, |second| {
                        TouchFrame::Pair([contact, second])
                    }),
            );
        }
        self.pending = Some(contact);
        None
    }
}
