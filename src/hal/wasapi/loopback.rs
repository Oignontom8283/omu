//! Per-application capture: WASAPI "process loopback" (`VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK`).
//!
//! It captures what a process and its children play, before the mix reaches any device, so the application can
//! stay on the user's speakers and still be copied. The activation is asynchronous: a completion handler (which
//! must be free-threaded, hence `IAgileObject`) signals an event.
//!
//! Known behaviours (from Microsoft's sample and field reports): the resulting client has no mix format, so the
//! format is chosen by us; an application that plays nothing produces no packets; Windows 10 builds before the
//! feature shipped answer with an error, reported as "not supported".

use super::com::{describe, ComGuard};
use super::stream::Event;
use crate::hal::HalError;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::time::Duration;
use windows::core::{implement, Interface, Ref};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioClient, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{IAgileObject, IAgileObject_Impl, BLOB};
use windows::Win32::System::Variant::VT_BLOB;

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Handler {
    done: Arc<Event>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Handler_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        self.done.signal();
        Ok(())
    }
}

impl IAgileObject_Impl for Handler_Impl {}

/// Opens an (uninitialised) client that captures the tree of process `pid`.
pub fn activate(pid: u32) -> Result<IAudioClient, HalError> {
    let _com = ComGuard::new();
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // The blob points into this stack frame: the PROPVARIANT must never be dropped (its destructor would hand the
    // stack pointer to CoTaskMemFree and corrupt the heap).
    let propvariant = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: &params as *const AUDIOCLIENT_ACTIVATION_PARAMS as *mut u8,
                    },
                },
            }),
        },
    });
    let done = Arc::new(Event::new()?);
    let handler: IActivateAudioInterfaceCompletionHandler = Handler { done: done.clone() }.into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*propvariant),
            &handler,
        )
    }
    .map_err(|e| {
        HalError::Unsupported(format!(
            "capturing one application needs a recent Windows: {}",
            describe("ActivateAudioInterfaceAsync", &e)
        ))
    })?;
    if !done.wait(Duration::from_secs(5)) {
        return Err(HalError::Backend(
            "ActivateAudioInterfaceAsync did not complete within 5 seconds".into(),
        ));
    }
    let mut result = windows::core::HRESULT(0);
    let mut unknown = None;
    unsafe { operation.GetActivateResult(&mut result, &mut unknown) }
        .map_err(|e| HalError::Backend(describe("GetActivateResult", &e)))?;
    if result.is_err() {
        return Err(HalError::Busy(format!(
            "Windows refused to capture process {pid}: {}",
            describe("activation", &windows::core::Error::from(result))
        )));
    }
    unknown
        .ok_or_else(|| HalError::Backend("the activation returned no audio client".into()))?
        .cast::<IAudioClient>()
        .map_err(|e| HalError::Backend(describe("cast(IAudioClient)", &e)))
}
