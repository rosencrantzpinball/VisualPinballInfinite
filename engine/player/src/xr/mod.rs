//! VR through OpenXR (Linux: the Steam Frame with SteamVR's runtime, and Monado's simulated headset in
//! tests): the runtime, the session, a swapchain for each eye and the frame loop's timing and views.
//!
//! The Khronos loader (`libopenxr_loader.so.1`) is loaded when VR starts: beside the player, else in a
//! development build our own build of it (`tools/openxr/setup.sh`), else the system's. Drawing goes
//! through `XR_KHR_vulkan_enable2`: the runtime makes SDL_GPU's Vulkan instance and device, on the
//! physical device it names (the vendored SDL's creation hooks), and the renderer draws each eye into the
//! runtime's swapchain images, which SDL wraps as textures. Everything runs on the thread that draws. Ours.

pub mod controls;
pub mod fovea;
pub mod place;

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use openxr_sys as xr;
use openxr_sys::Handle as _;
use sdl3_sys::everything::*;

use place::{Fov, Pose};

const VULKAN_ENABLE2: &CStr = c"XR_KHR_vulkan_enable2";
/// The Steam Frame's controllers (SteamVR); without it the runtime presents them as Touch controllers.
pub const VALVE_FRAME_CONTROLLER: &CStr = c"XR_VALVE_frame_controller_interaction";
/// The runtime's counters, its GPU time for the app's frames among them (SteamVR on the Frame).
const PERFORMANCE_METRICS: &CStr = c"XR_META_performance_metrics";
/// Foveated rendering (fovea.rs): an eye-tracked foveation profile, which turns the eye tracker on, the
/// runtime's foveation centres, and the gaze as an input.
const FB_FOVEATION: &CStr = c"XR_FB_foveation";
const FB_FOVEATION_CONFIGURATION: &CStr = c"XR_FB_foveation_configuration";
const FB_SWAPCHAIN_UPDATE_STATE: &CStr = c"XR_FB_swapchain_update_state";
const FB_FOVEATION_VULKAN: &CStr = c"XR_FB_foveation_vulkan";
const META_FOVEATION_EYE_TRACKED: &CStr = c"XR_META_foveation_eye_tracked";
const EXT_EYE_GAZE: &CStr = c"XR_EXT_eye_gaze_interaction";

/// The vendored SDL's `SDL_GPUVulkanCreationHooks` (`SDL_gpu.h`, ours).
#[repr(C)]
struct CreationHooks {
    userdata: *mut c_void,
    create_instance: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void, *mut c_void) -> c_int,
    physical_device: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    create_device: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const c_void, *mut c_void) -> c_int,
}

/// SDL_GPUVulkanOptions (SDL 3.4's), for the Vulkan version the runtime asks for.
#[repr(C)]
struct VulkanOptions {
    vulkan_api_version: u32,
    feature_list: *mut c_void,
    vulkan_10_physical_device_features: *mut c_void,
    device_extension_count: u32,
    device_extension_names: *const *const c_char,
    instance_extension_count: u32,
    instance_extension_names: *const *const c_char,
}

const CREATION_HOOKS: &CStr = c"SDL.gpu.device.create.vulkan.creation_hooks";
const VULKAN_INSTANCE: &CStr = c"SDL.gpu.device.vulkan.instance";
const VULKAN_PHYSICAL_DEVICE: &CStr = c"SDL.gpu.device.vulkan.physical_device";
const VULKAN_DEVICE: &CStr = c"SDL.gpu.device.vulkan.device";
const VULKAN_QUEUE_FAMILY: &CStr = c"SDL.gpu.device.vulkan.queue_family_index";
const VULKAN_QUEUE_INDEX: &CStr = c"SDL.gpu.device.vulkan.queue_index";
const TEXTURE_VULKAN_IMAGE: &CStr = c"SDL.gpu.texture.create.vulkan.image";
/// The vendored SDL's fragment shading rate images: asked for with the device, then its texel (0: none).
const VULKAN_SHADING_RATE: &CStr = c"SDL.gpu.device.create.vulkan.fragment_shading_rate";
pub const VULKAN_SHADING_RATE_TEXEL: &CStr = c"SDL.gpu.device.vulkan.shading_rate_texel_size";

macro_rules! functions {
    ($($field:ident: $pfn:ident),* $(,)?) => {
        /// The runtime's functions the player calls.
        #[allow(non_snake_case)]
        struct Functions {
            $($field: xr::pfn::$pfn,)*
        }

        impl Functions {
            unsafe fn load(get: xr::pfn::GetInstanceProcAddr, instance: xr::Instance) -> Result<Functions, String> {
                Ok(Functions { $($field: unsafe { function(get, instance, concat!("xr", stringify!($pfn)))? },)* })
            }
        }
    };
}

functions! {
    destroy_instance: DestroyInstance,
    get_system: GetSystem,
    get_system_properties: GetSystemProperties,
    enumerate_view_configuration_views: EnumerateViewConfigurationViews,
    enumerate_environment_blend_modes: EnumerateEnvironmentBlendModes,
    get_vulkan_graphics_requirements2: GetVulkanGraphicsRequirements2KHR,
    create_vulkan_instance: CreateVulkanInstanceKHR,
    get_vulkan_graphics_device2: GetVulkanGraphicsDevice2KHR,
    create_vulkan_device: CreateVulkanDeviceKHR,
    create_session: CreateSession,
    destroy_session: DestroySession,
    begin_session: BeginSession,
    end_session: EndSession,
    request_exit_session: RequestExitSession,
    enumerate_reference_spaces: EnumerateReferenceSpaces,
    create_reference_space: CreateReferenceSpace,
    destroy_space: DestroySpace,
    enumerate_swapchain_formats: EnumerateSwapchainFormats,
    create_swapchain: CreateSwapchain,
    destroy_swapchain: DestroySwapchain,
    enumerate_swapchain_images: EnumerateSwapchainImages,
    acquire_swapchain_image: AcquireSwapchainImage,
    wait_swapchain_image: WaitSwapchainImage,
    release_swapchain_image: ReleaseSwapchainImage,
    poll_event: PollEvent,
    wait_frame: WaitFrame,
    begin_frame: BeginFrame,
    end_frame: EndFrame,
    locate_views: LocateViews,
    result_to_string: ResultToString,
    string_to_path: StringToPath,
    path_to_string: PathToString,
    create_action_set: CreateActionSet,
    destroy_action_set: DestroyActionSet,
    create_action: CreateAction,
    suggest_interaction_profile_bindings: SuggestInteractionProfileBindings,
    attach_session_action_sets: AttachSessionActionSets,
    sync_actions: SyncActions,
    get_action_state_boolean: GetActionStateBoolean,
    get_action_state_float: GetActionStateFloat,
    get_action_state_vector2f: GetActionStateVector2f,
    get_current_interaction_profile: GetCurrentInteractionProfile,
    create_action_space: CreateActionSpace,
    locate_space: LocateSpace,
}

/// A function of the runtime's, by name.
unsafe fn function<T>(get: xr::pfn::GetInstanceProcAddr, instance: xr::Instance, name: &str) -> Result<T, String> {
    let c = CString::new(name).unwrap_or_default();
    let mut f: Option<xr::pfn::VoidFunction> = None;
    let r = unsafe { get(instance, c.as_ptr(), &mut f) };
    match f {
        Some(f) if r == xr::Result::SUCCESS => Ok(unsafe { std::mem::transmute_copy::<xr::pfn::VoidFunction, T>(&f) }),
        _ => Err(format!("the OpenXR runtime has no {name} ({r:?})")),
    }
}

/// Where the loader is looked for.
fn loader_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.join("libopenxr_loader.so.1"));
        if crate::DEVELOPMENT {
            out.extend(dir.ancestors().take(4).map(|up| up.join("vpx-build").join("linux-aarch64").join("openxr").join("libopenxr_loader.so.1")));
        }
    }
    // the system's, found by the dynamic linker
    out.push(PathBuf::from("libopenxr_loader.so.1"));
    out
}

unsafe extern "C" {
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}
const RTLD_NOW: c_int = 2;

/// The OpenXR runtime: the loader, the instance and the headset (the system), from before the GPU
/// device exists, which the runtime makes.
pub struct Runtime {
    f: Functions,
    get: xr::pfn::GetInstanceProcAddr,
    instance: xr::Instance,
    system: xr::SystemId,
    /// The runtime's name and version, for the log.
    pub name: String,
    /// The headset's name.
    pub headset: String,
    /// Each eye's recommended and largest image sizes.
    pub views: [xr::ViewConfigurationView; 2],
    /// The extensions the instance has.
    pub extensions: Vec<String>,
    /// How the runtime blends our picture with the world (`XR_ENVIRONMENT_BLEND_MODE_*`).
    pub blend: xr::EnvironmentBlendMode,
    /// Foveated rendering asked for: the device made with fragment shading rate images, the eye tracking
    /// extensions taken.
    foveation: bool,
    /// The headset's eye tracker places the runtime's foveation centres (`XR_META_foveation_eye_tracked`).
    pub eye_tracked_foveation: bool,
    /// The gaze is an input (`XR_EXT_eye_gaze_interaction`).
    pub eye_gaze: bool,
    vulkan_version: u32,
    /// The hooks SDL calls while it makes the device: they point here, so the runtime is boxed.
    hooks: CreationHooks,
}

impl Runtime {
    /// Starts OpenXR: the loader, an instance with Vulkan (and the extensions the player can use that
    /// the runtime has), and the headset. `passthrough` asks for the world to show where nothing is
    /// drawn (alpha blend), where the runtime can; `foveation` for foveated rendering's.
    pub fn start(passthrough: bool, foveation: bool) -> Result<Box<Runtime>, String> {
        // SAFETY: loading the loader and calling OpenXR's functions as its specification says
        unsafe {
            let library = loader_candidates()
                .into_iter()
                .find_map(|p| {
                    let c = CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
                    let h = dlopen(c.as_ptr(), RTLD_NOW);
                    (!h.is_null()).then_some(h)
                })
                .ok_or("the OpenXR loader (libopenxr_loader.so.1) wasn't found")?;
            let get = dlsym(library, c"xrGetInstanceProcAddr".as_ptr());
            if get.is_null() {
                return Err("the OpenXR loader has no xrGetInstanceProcAddr".into());
            }
            let get: xr::pfn::GetInstanceProcAddr = std::mem::transmute(get);
            let enumerate: xr::pfn::EnumerateInstanceExtensionProperties = function(get, xr::Instance::NULL, "xrEnumerateInstanceExtensionProperties")?;
            let create: xr::pfn::CreateInstance = function(get, xr::Instance::NULL, "xrCreateInstance")?;

            let mut count = 0;
            enumerate(null(), 0, &mut count, null_mut());
            let mut props = vec![xr::ExtensionProperties { ty: xr::StructureType::EXTENSION_PROPERTIES, next: null_mut(), extension_name: [0; 128], extension_version: 0 }; count as usize];
            enumerate(null(), count, &mut count, props.as_mut_ptr());
            let available: Vec<String> = props.iter().map(|p| CStr::from_ptr(p.extension_name.as_ptr()).to_string_lossy().into_owned()).collect();
            let has = |e: &CStr| available.iter().any(|a| a.as_bytes() == e.to_bytes());
            if !has(VULKAN_ENABLE2) {
                return Err("the OpenXR runtime can't draw with Vulkan (no XR_KHR_vulkan_enable2)".into());
            }
            let mut offered = vec![VULKAN_ENABLE2, VALVE_FRAME_CONTROLLER, PERFORMANCE_METRICS];
            if foveation {
                offered.extend([FB_FOVEATION, FB_FOVEATION_CONFIGURATION, FB_SWAPCHAIN_UPDATE_STATE, FB_FOVEATION_VULKAN, META_FOVEATION_EYE_TRACKED, EXT_EYE_GAZE]);
            }
            let wanted: Vec<&CStr> = offered.into_iter().filter(|e| has(e)).collect();
            let names: Vec<*const c_char> = wanted.iter().map(|e| e.as_ptr()).collect();

            let mut app = xr::ApplicationInfo {
                application_name: [0; xr::MAX_APPLICATION_NAME_SIZE],
                application_version: 1,
                engine_name: [0; xr::MAX_ENGINE_NAME_SIZE],
                engine_version: 1,
                // VPX's: 1.0, which every runtime takes
                api_version: xr::Version::new(1, 0, 0),
            };
            for (dst, src) in [(&mut app.application_name[..], "Visual Pinball Infinite"), (&mut app.engine_name[..], "Visual Pinball Infinite")] {
                for (d, s) in dst.iter_mut().zip(src.bytes()) {
                    *d = s as c_char;
                }
            }
            let info = xr::InstanceCreateInfo {
                ty: xr::StructureType::INSTANCE_CREATE_INFO,
                next: null(),
                create_flags: xr::InstanceCreateFlags::EMPTY,
                application_info: app,
                enabled_api_layer_count: 0,
                enabled_api_layer_names: null(),
                enabled_extension_count: names.len() as u32,
                enabled_extension_names: names.as_ptr(),
            };
            let mut instance = xr::Instance::NULL;
            let r = create(&info, &mut instance);
            if r != xr::Result::SUCCESS {
                return Err(format!("the OpenXR instance couldn't be made ({r:?})"));
            }
            let f = match Functions::load(get, instance) {
                Ok(f) => f,
                Err(e) => {
                    if let Ok(destroy) = function::<xr::pfn::DestroyInstance>(get, instance, "xrDestroyInstance") {
                        destroy(instance);
                    }
                    return Err(e);
                }
            };
            let mut runtime = Box::new(Runtime {
                f,
                get,
                instance,
                system: xr::SystemId::NULL,
                name: String::new(),
                headset: String::new(),
                views: [xr::ViewConfigurationView {
                    ty: xr::StructureType::VIEW_CONFIGURATION_VIEW,
                    next: null_mut(),
                    recommended_image_rect_width: 0,
                    max_image_rect_width: 0,
                    recommended_image_rect_height: 0,
                    max_image_rect_height: 0,
                    recommended_swapchain_sample_count: 0,
                    max_swapchain_sample_count: 0,
                }; 2],
                extensions: wanted.iter().map(|e| e.to_string_lossy().into_owned()).collect(),
                blend: xr::EnvironmentBlendMode::OPAQUE,
                foveation,
                eye_tracked_foveation: false,
                eye_gaze: false,
                vulkan_version: 0,
                hooks: CreationHooks { userdata: null_mut(), create_instance, physical_device, create_device },
            });
            runtime.name = runtime_name(get, instance);
            runtime.find_headset(passthrough)?;
            Ok(runtime)
        }
    }

    /// The headset, its eyes' views, the blend mode and the Vulkan version the runtime wants.
    unsafe fn find_headset(&mut self, passthrough: bool) -> Result<(), String> {
        unsafe {
            let info = xr::SystemGetInfo { ty: xr::StructureType::SYSTEM_GET_INFO, next: null(), form_factor: xr::FormFactor::HEAD_MOUNTED_DISPLAY };
            let r = (self.f.get_system)(self.instance, &info, &mut self.system);
            if r != xr::Result::SUCCESS {
                return Err(format!("no headset ({})", self.result(r)));
            }
            let mut props: xr::SystemProperties = std::mem::zeroed();
            props.ty = xr::StructureType::SYSTEM_PROPERTIES;
            // the eye tracking extensions' own, when the instance has them
            let mut eye_tracked: xr::SystemFoveationEyeTrackedPropertiesMETA = std::mem::zeroed();
            eye_tracked.ty = xr::StructureType::SYSTEM_FOVEATION_EYE_TRACKED_PROPERTIES_META;
            let mut gaze: xr::SystemEyeGazeInteractionPropertiesEXT = std::mem::zeroed();
            gaze.ty = xr::StructureType::SYSTEM_EYE_GAZE_INTERACTION_PROPERTIES_EXT;
            if self.has(EXT_EYE_GAZE) {
                gaze.next = props.next;
                props.next = (&raw mut gaze).cast();
            }
            if self.has(META_FOVEATION_EYE_TRACKED) {
                eye_tracked.next = props.next;
                props.next = (&raw mut eye_tracked).cast();
            }
            if (self.f.get_system_properties)(self.instance, self.system, &mut props) == xr::Result::SUCCESS {
                self.headset = CStr::from_ptr(props.system_name.as_ptr()).to_string_lossy().into_owned();
                self.eye_tracked_foveation = self.has(META_FOVEATION_EYE_TRACKED) && eye_tracked.supports_foveation_eye_tracked == xr::TRUE;
                self.eye_gaze = self.has(EXT_EYE_GAZE) && gaze.supports_eye_gaze_interaction == xr::TRUE;
            }
            let mut count = 0;
            let stereo = xr::ViewConfigurationType::PRIMARY_STEREO;
            (self.f.enumerate_view_configuration_views)(self.instance, self.system, stereo, 0, &mut count, null_mut());
            if count != 2 {
                return Err(format!("the headset has {count} views, not 2"));
            }
            let r = (self.f.enumerate_view_configuration_views)(self.instance, self.system, stereo, 2, &mut count, self.views.as_mut_ptr());
            check(self, r, "the eyes' views")?;
            (self.f.enumerate_environment_blend_modes)(self.instance, self.system, stereo, 0, &mut count, null_mut());
            let mut modes = vec![xr::EnvironmentBlendMode::OPAQUE; count as usize];
            (self.f.enumerate_environment_blend_modes)(self.instance, self.system, stereo, count, &mut count, modes.as_mut_ptr());
            // the world shows through where nothing is drawn, as the owner's VPX fork does it on the
            // Frame, when asked and the runtime can; else opaque, as VPX (`VPI_XR_BLEND` picks one)
            let asked = match std::env::var("VPI_XR_BLEND").as_deref() {
                Ok("opaque") => Some(xr::EnvironmentBlendMode::OPAQUE),
                Ok("alpha") => Some(xr::EnvironmentBlendMode::ALPHA_BLEND),
                Ok("additive") => Some(xr::EnvironmentBlendMode::ADDITIVE),
                _ => None,
            };
            let preferred = if passthrough { xr::EnvironmentBlendMode::ALPHA_BLEND } else { xr::EnvironmentBlendMode::OPAQUE };
            self.blend = [asked, Some(preferred), Some(xr::EnvironmentBlendMode::OPAQUE)]
                .into_iter()
                .flatten()
                .find(|m| modes.contains(m))
                .unwrap_or_else(|| modes.first().copied().unwrap_or(xr::EnvironmentBlendMode::OPAQUE));
            let mut req = xr::GraphicsRequirementsVulkanKHR {
                ty: xr::StructureType::GRAPHICS_REQUIREMENTS_VULKAN_KHR,
                next: null_mut(),
                min_api_version_supported: xr::Version::new(1, 0, 0),
                max_api_version_supported: xr::Version::new(1, 0, 0),
            };
            check(self, (self.f.get_vulkan_graphics_requirements2)(self.instance, self.system, &mut req), "the runtime's Vulkan requirements")?;
            // at least Vulkan 1.1 for the device's features, within what the runtime takes
            let min = req.min_api_version_supported;
            let (major, minor) = if (min.major(), min.minor()) > (1, 1) { (min.major(), min.minor()) } else { (1, 1) };
            self.vulkan_version = (u32::from(major) << 22) | (u32::from(minor) << 12);
            Ok(())
        }
    }

    /// Makes the GPU device through the runtime (`XR_KHR_vulkan_enable2`): SDL_GPU on Vulkan, its
    /// instance and device made by the runtime, on the physical device it names.
    ///
    /// # Safety
    /// On the thread that draws; SDL's video started; the runtime outlives the device.
    pub unsafe fn create_device(&mut self) -> Result<*mut SDL_GPUDevice, String> {
        unsafe {
            self.hooks.userdata = (self as *mut Runtime).cast();
            let options = VulkanOptions {
                vulkan_api_version: self.vulkan_version,
                feature_list: null_mut(),
                vulkan_10_physical_device_features: null_mut(),
                device_extension_count: 0,
                device_extension_names: null(),
                instance_extension_count: 0,
                instance_extension_names: null(),
            };
            let props = SDL_CreateProperties();
            SDL_SetStringProperty(props, SDL_PROP_GPU_DEVICE_CREATE_NAME_STRING, c"vulkan".as_ptr());
            SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN, true);
            SDL_SetBooleanProperty(props, SDL_PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN, false);
            SDL_SetPointerProperty(props, SDL_PROP_GPU_DEVICE_CREATE_VULKAN_OPTIONS_POINTER, (&raw const options).cast_mut().cast());
            SDL_SetPointerProperty(props, CREATION_HOOKS.as_ptr(), (&raw mut self.hooks).cast());
            // foveated rendering's fragment shading rate images, where the GPU has them
            SDL_SetBooleanProperty(props, VULKAN_SHADING_RATE.as_ptr(), self.foveation);
            let device = SDL_CreateGPUDeviceWithProperties(props);
            SDL_DestroyProperties(props);
            if device.is_null() {
                return Err(format!("the GPU device for the headset couldn't be made: {}", crate::render::sdl_error()));
            }
            Ok(device)
        }
    }

    pub fn result(&self, r: xr::Result) -> String {
        let mut buf = [0 as c_char; xr::MAX_RESULT_STRING_SIZE];
        // SAFETY: a buffer of the size OpenXR asks
        unsafe {
            if (self.f.result_to_string)(self.instance, r, buf.as_mut_ptr()) == xr::Result::SUCCESS {
                return CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
            }
        }
        format!("{r:?}")
    }

    /// Whether the instance has an extension.
    pub fn has(&self, e: &CStr) -> bool {
        self.extensions.iter().any(|x| x.as_bytes() == e.to_bytes())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: our instance, destroyed once, after its session and the GPU device it made
        unsafe { (self.f.destroy_instance)(self.instance) };
    }
}

fn check(rt: &Runtime, r: xr::Result, what: &str) -> Result<(), String> {
    if r == xr::Result::SUCCESS { Ok(()) } else { Err(format!("{what}: {}", rt.result(r))) }
}

/// The runtime's name and version.
unsafe fn runtime_name(get: xr::pfn::GetInstanceProcAddr, instance: xr::Instance) -> String {
    unsafe {
        let Ok(f) = function::<xr::pfn::GetInstanceProperties>(get, instance, "xrGetInstanceProperties") else { return String::new() };
        let mut p: xr::InstanceProperties = std::mem::zeroed();
        p.ty = xr::StructureType::INSTANCE_PROPERTIES;
        if f(instance, &mut p) != xr::Result::SUCCESS {
            return String::new();
        }
        let v = p.runtime_version;
        format!("{} {}.{}.{}", CStr::from_ptr(p.runtime_name.as_ptr()).to_string_lossy(), v.major(), v.minor(), v.patch())
    }
}

/// SDL's hook: the Vulkan instance, made by the runtime (`xrCreateVulkanInstanceKHR`).
unsafe extern "C" fn create_instance(userdata: *mut c_void, get_proc: *mut c_void, info: *const c_void, out: *mut c_void) -> c_int {
    // SAFETY: SDL passes what the hooks' userdata was set to, a Vulkan instance's create info and
    // where the instance goes
    unsafe {
        let rt = &*(userdata as *const Runtime);
        let ci = xr::VulkanInstanceCreateInfoKHR {
            ty: xr::StructureType::VULKAN_INSTANCE_CREATE_INFO_KHR,
            next: null(),
            system_id: rt.system,
            create_flags: xr::VulkanInstanceCreateFlagsKHR::EMPTY,
            pfn_get_instance_proc_addr: Some(std::mem::transmute::<*mut c_void, xr::platform::VkGetInstanceProcAddr>(get_proc)),
            vulkan_create_info: info,
            vulkan_allocator: null(),
        };
        let mut instance: xr::platform::VkInstance = null();
        let mut vk: xr::platform::VkResult = 0;
        let r = (rt.f.create_vulkan_instance)(rt.instance, &ci, &mut instance, &mut vk);
        if r != xr::Result::SUCCESS {
            eprintln!("VR: the runtime couldn't make the Vulkan instance: {}", rt.result(r));
            return -3; // VK_ERROR_INITIALIZATION_FAILED
        }
        *(out as *mut xr::platform::VkInstance) = instance;
        vk
    }
}

/// SDL's hook: the physical device the runtime draws with (`xrGetVulkanGraphicsDevice2KHR`).
unsafe extern "C" fn physical_device(userdata: *mut c_void, instance: *mut c_void) -> *mut c_void {
    // SAFETY: as above, with the Vulkan instance the runtime made
    unsafe {
        let rt = &*(userdata as *const Runtime);
        let info = xr::VulkanGraphicsDeviceGetInfoKHR {
            ty: xr::StructureType::VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR,
            next: null(),
            system_id: rt.system,
            vulkan_instance: instance,
        };
        let mut device: xr::platform::VkPhysicalDevice = null();
        let r = (rt.f.get_vulkan_graphics_device2)(rt.instance, &info, &mut device);
        if r != xr::Result::SUCCESS {
            eprintln!("VR: the runtime named no Vulkan device: {}", rt.result(r));
            return null_mut();
        }
        device.cast_mut()
    }
}

/// SDL's hook: the Vulkan device, made by the runtime (`xrCreateVulkanDeviceKHR`).
unsafe extern "C" fn create_device(userdata: *mut c_void, get_proc: *mut c_void, physical: *mut c_void, info: *const c_void, out: *mut c_void) -> c_int {
    // SAFETY: as above
    unsafe {
        let rt = &*(userdata as *const Runtime);
        let ci = xr::VulkanDeviceCreateInfoKHR {
            ty: xr::StructureType::VULKAN_DEVICE_CREATE_INFO_KHR,
            next: null(),
            system_id: rt.system,
            create_flags: xr::VulkanDeviceCreateFlagsKHR::EMPTY,
            pfn_get_instance_proc_addr: Some(std::mem::transmute::<*mut c_void, xr::platform::VkGetInstanceProcAddr>(get_proc)),
            vulkan_physical_device: physical,
            vulkan_create_info: info,
            vulkan_allocator: null(),
        };
        let mut device: xr::platform::VkDevice = null();
        let mut vk: xr::platform::VkResult = 0;
        let r = (rt.f.create_vulkan_device)(rt.instance, &ci, &mut device, &mut vk);
        if r != xr::Result::SUCCESS {
            eprintln!("VR: the runtime couldn't make the Vulkan device: {}", rt.result(r));
            return -3;
        }
        *(out as *mut xr::platform::VkDevice) = device;
        vk
    }
}

/// An eye's swapchain: its images, each wrapped as an SDL texture the renderer draws into.
struct Swapchain {
    handle: xr::Swapchain,
    width: u32,
    height: u32,
    textures: Vec<*mut SDL_GPUTexture>,
    /// The image acquired for this frame.
    acquired: Option<usize>,
}

/// The quad layers over the eyes': the headset's panel (the app's screens, `crate::panel`) and the
/// pause menu, each a panel in the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Panel,
    Menu,
}

impl Layer {
    fn name(self) -> &'static str {
        match self {
            Layer::Panel => "panel",
            Layer::Menu => "pause menu",
        }
    }
}

/// A quad's two swapchains, of one size, its pictures uploaded as they change: the layer shows one
/// while a new picture is written into the other's next image, and they change places once the GPU
/// has written it. SteamVR draws quads in its lens pass, as the headset shows them: with one swapchain,
/// the image being written could be the one it read, which showed in the headset as blocks of black and
/// white over the panel, never in SteamVR's recordings of it.
struct QuadChain {
    chains: [Swapchain; 2],
    /// The one the layer shows; the other is written.
    front: usize,
    /// Its images are BGRA (else RGBA): a picture in the other order has its red and blue swapped.
    bgra: bool,
    transfer: *mut SDL_GPUTransferBuffer,
    /// The GPU's copy into the other's image while it's written (null: none), which is handed over and
    /// shown once the copy is done (`quad_settle`).
    writing: *mut SDL_GPUFence,
    /// An image was handed over: the layer has something to show.
    shown: bool,
}

/// A quad this frame: its pose in the room's space, its size in metres, and the part of its
/// images its pictures fill.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub pose: Pose,
    pub size: (f32, f32),
    pub rect: (u32, u32),
}

/// The quads' formats, preferred first: sRGB, as their pictures are, BGRA as the panel's come (and
/// whether it's BGRA).
const QUAD_FORMATS: [(i64, SDL_GPUTextureFormat, bool); 2] = [
    (50, SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB, true), // VK_FORMAT_B8G8R8A8_SRGB
    (43, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB, false), // VK_FORMAT_R8G8B8A8_SRGB
];

/// The session: the headset showing what's drawn, in a space of the room.
pub struct Session {
    session: xr::Session,
    space: xr::Space,
    /// The space is the room's floor (a stage space), as VPX's placement wants; else the runtime's own.
    pub floor: bool,
    device: *mut SDL_GPUDevice,
    swapchains: Vec<Swapchain>,
    /// The textures' format.
    pub format: SDL_GPUTextureFormat,
    state: xr::SessionState,
    running: bool,
    /// The session has ended, or the runtime asked the player to quit.
    pub exit: bool,
    /// The runtime's GPU time counter for our frames (`XR_META_performance_metrics`), when it has one.
    gpu_time: Option<GpuTime>,
    /// Whether the headset shows the picture (visible or focused) and has the controllers (focused).
    pub visible: bool,
    pub focused: bool,
    /// The runtime's foveation centres, once an eye-tracked profile turned the eye tracker on.
    eye_tracked_state: Option<xr::pfn::GetFoveationEyeTrackedStateMETA>,
    panel: Option<QuadChain>,
    menu: Option<QuadChain>,
}

/// The swapchain formats the renderer can draw into, preferred first: sRGB, so the linear picture is
/// encoded by the GPU as on the desktop's swapchain (VPX's own preference, R8G8B8A8 sRGB, first).
const FORMATS: [(i64, SDL_GPUTextureFormat); 4] = [
    (43, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB), // VK_FORMAT_R8G8B8A8_SRGB
    (50, SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB), // VK_FORMAT_B8G8R8A8_SRGB
    (37, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM),      // VK_FORMAT_R8G8B8A8_UNORM
    (44, SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM),      // VK_FORMAT_B8G8R8A8_UNORM
];

impl Session {
    /// Begins a session on `device` (made by `Runtime::create_device`), each eye's swapchain
    /// `size` pixels (the runtime's recommended size, or the player's resolution).
    ///
    /// # Safety
    /// On the thread that draws; the device lives until the session is dropped.
    pub unsafe fn new(rt: &Runtime, device: *mut SDL_GPUDevice, size: (u32, u32)) -> Result<Session, String> {
        unsafe {
            let props = SDL_GetGPUDeviceProperties(device);
            let binding = xr::GraphicsBindingVulkanKHR {
                ty: xr::StructureType::GRAPHICS_BINDING_VULKAN_KHR,
                next: null(),
                instance: SDL_GetPointerProperty(props, VULKAN_INSTANCE.as_ptr(), null_mut()),
                physical_device: SDL_GetPointerProperty(props, VULKAN_PHYSICAL_DEVICE.as_ptr(), null_mut()),
                device: SDL_GetPointerProperty(props, VULKAN_DEVICE.as_ptr(), null_mut()),
                queue_family_index: SDL_GetNumberProperty(props, VULKAN_QUEUE_FAMILY.as_ptr(), 0) as u32,
                queue_index: SDL_GetNumberProperty(props, VULKAN_QUEUE_INDEX.as_ptr(), 0) as u32,
            };
            if binding.device.is_null() {
                return Err("the GPU device isn't SDL's Vulkan one with our handles (the vendored SDL's)".into());
            }
            let info = xr::SessionCreateInfo { ty: xr::StructureType::SESSION_CREATE_INFO, next: (&raw const binding).cast(), create_flags: xr::SessionCreateFlags::EMPTY, system_id: rt.system };
            let mut session = xr::Session::NULL;
            check(rt, (rt.f.create_session)(rt.instance, &info, &mut session), "the VR session")?;
            let mut s = Session {
                session,
                space: xr::Space::NULL,
                floor: false,
                device,
                swapchains: Vec::new(),
                format: SDL_GPU_TEXTUREFORMAT_INVALID,
                state: xr::SessionState::UNKNOWN,
                running: false,
                exit: false,
                gpu_time: None,
                visible: false,
                focused: false,
                eye_tracked_state: None,
                panel: None,
                menu: None,
            };
            s.make_space(rt)?;
            s.make_swapchains(rt, size)?;
            s.gpu_time = GpuTime::enable(rt, session);
            s.eye_tracked_state = s.track_eyes(rt);
            Ok(s)
        }
    }

    /// The room's space: the stage (its floor at 0), as VPX asks, else the local space.
    unsafe fn make_space(&mut self, rt: &Runtime) -> Result<(), String> {
        unsafe {
            let mut count = 0;
            (rt.f.enumerate_reference_spaces)(self.session, 0, &mut count, null_mut());
            let mut kinds = vec![xr::ReferenceSpaceType::VIEW; count as usize];
            (rt.f.enumerate_reference_spaces)(self.session, count, &mut count, kinds.as_mut_ptr());
            self.floor = kinds.contains(&xr::ReferenceSpaceType::STAGE);
            let info = xr::ReferenceSpaceCreateInfo {
                ty: xr::StructureType::REFERENCE_SPACE_CREATE_INFO,
                next: null(),
                reference_space_type: if self.floor { xr::ReferenceSpaceType::STAGE } else { xr::ReferenceSpaceType::LOCAL },
                pose_in_reference_space: xr::Posef { orientation: xr::Quaternionf { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }, position: xr::Vector3f { x: 0.0, y: 0.0, z: 0.0 } },
            };
            check(rt, (rt.f.create_reference_space)(self.session, &info, &mut self.space), "the room's space")
        }
    }

    unsafe fn make_swapchains(&mut self, rt: &Runtime, (width, height): (u32, u32)) -> Result<(), String> {
        unsafe {
            let mut count = 0;
            (rt.f.enumerate_swapchain_formats)(self.session, 0, &mut count, null_mut());
            let mut formats = vec![0i64; count as usize];
            (rt.f.enumerate_swapchain_formats)(self.session, count, &mut count, formats.as_mut_ptr());
            let Some(&(vk_format, format)) = FORMATS.iter().find(|(f, _)| formats.contains(f)) else {
                return Err(format!("the runtime offers no swapchain format the player can draw into ({formats:?})"));
            };
            self.format = format;
            // swapchains a foveation profile can be applied to (the eye tracker's way on), as the fork makes them
            let foveation = xr::SwapchainCreateInfoFoveationFB {
                ty: xr::StructureType::SWAPCHAIN_CREATE_INFO_FOVEATION_FB,
                next: null_mut(),
                flags: if rt.has(FB_FOVEATION_VULKAN) { xr::SwapchainCreateFoveationFlagsFB::FRAGMENT_DENSITY_MAP } else { xr::SwapchainCreateFoveationFlagsFB::EMPTY },
            };
            for _eye in 0..2 {
                let info = xr::SwapchainCreateInfo {
                    ty: xr::StructureType::SWAPCHAIN_CREATE_INFO,
                    next: if rt.has(FB_FOVEATION) { (&raw const foveation).cast() } else { null() },
                    create_flags: xr::SwapchainCreateFlags::EMPTY,
                    // drawn into; copied from for the test runs' pictures
                    usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::TRANSFER_SRC,
                    format: vk_format,
                    sample_count: 1,
                    width,
                    height,
                    face_count: 1,
                    array_size: 1,
                    mip_count: 1,
                };
                let mut handle = xr::Swapchain::NULL;
                check(rt, (rt.f.create_swapchain)(self.session, &info, &mut handle), "an eye's swapchain")?;
                let mut n = 0;
                (rt.f.enumerate_swapchain_images)(handle, 0, &mut n, null_mut());
                let mut images = vec![xr::SwapchainImageVulkanKHR { ty: xr::StructureType::SWAPCHAIN_IMAGE_VULKAN_KHR, next: null_mut(), image: 0 }; n as usize];
                let r = (rt.f.enumerate_swapchain_images)(handle, n, &mut n, images.as_mut_ptr().cast());
                let mut chain = Swapchain { handle, width, height, textures: Vec::new(), acquired: None };
                if r != xr::Result::SUCCESS {
                    self.swapchains.push(chain);
                    return Err(format!("an eye's swapchain images: {}", rt.result(r)));
                }
                for image in &images {
                    let props = SDL_CreateProperties();
                    SDL_SetNumberProperty(props, TEXTURE_VULKAN_IMAGE.as_ptr(), image.image as i64);
                    let mut ti: SDL_GPUTextureCreateInfo = std::mem::zeroed();
                    ti.r#type = SDL_GPU_TEXTURETYPE_2D;
                    ti.format = format;
                    ti.usage = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET;
                    ti.width = width;
                    ti.height = height;
                    ti.layer_count_or_depth = 1;
                    ti.num_levels = 1;
                    ti.sample_count = SDL_GPU_SAMPLECOUNT_1;
                    ti.props = props;
                    let texture = SDL_CreateGPUTexture(self.device, &ti);
                    SDL_DestroyProperties(props);
                    if texture.is_null() {
                        self.swapchains.push(chain);
                        return Err(format!("an eye's image as a texture: {}", crate::render::sdl_error()));
                    }
                    chain.textures.push(texture);
                }
                self.swapchains.push(chain);
            }
            Ok(())
        }
    }

    /// The GPU time the runtime counted for our last frames, in ms (`XR_META_performance_metrics`'s
    /// `/perfmetrics_meta/app/gpu_frametime`); about 0.2 ms while nothing is drawn.
    pub fn gpu_ms(&self) -> Option<f32> {
        let g = self.gpu_time.as_ref()?;
        // SAFETY: our session and the runtime's counter
        unsafe {
            let mut c: xr::PerformanceMetricsCounterMETA = std::mem::zeroed();
            c.ty = xr::StructureType::PERFORMANCE_METRICS_COUNTER_META;
            ((g.query)(self.session, g.path, &mut c) == xr::Result::SUCCESS && c.counter_flags.contains(xr::PerformanceMetricsCounterFlagsMETA::FLOAT_VALUE_VALID)).then_some(c.float_value)
        }
    }

    /// Each eye's image size.
    pub fn eye_size(&self) -> (u32, u32) {
        self.swapchains.first().map_or((0, 0), |s| (s.width, s.height))
    }

    /// Hears the runtime's events: the session's state (begun when ready, ended when stopping, over
    /// when exiting or lost), the controllers' profile. `events` hears each, for the controls.
    pub fn poll(&mut self, rt: &Runtime, mut events: impl FnMut(&xr::EventDataBuffer)) {
        // SAFETY: OpenXR's event queue, read into a buffer of its size
        unsafe {
            loop {
                let mut e: xr::EventDataBuffer = std::mem::zeroed();
                e.ty = xr::StructureType::EVENT_DATA_BUFFER;
                if (rt.f.poll_event)(rt.instance, &mut e) != xr::Result::SUCCESS {
                    break;
                }
                match e.ty {
                    xr::StructureType::EVENT_DATA_SESSION_STATE_CHANGED => {
                        let changed = &*(&raw const e).cast::<xr::EventDataSessionStateChanged>();
                        self.state = changed.state;
                        println!("VR: the session is {:?}", changed.state);
                        match changed.state {
                            xr::SessionState::READY => {
                                let info = xr::SessionBeginInfo {
                                    ty: xr::StructureType::SESSION_BEGIN_INFO,
                                    next: null(),
                                    primary_view_configuration_type: xr::ViewConfigurationType::PRIMARY_STEREO,
                                };
                                match check(rt, (rt.f.begin_session)(self.session, &info), "beginning the session") {
                                    Ok(()) => self.running = true,
                                    Err(e) => eprintln!("VR: {e}"),
                                }
                            }
                            xr::SessionState::STOPPING => {
                                (rt.f.end_session)(self.session);
                                self.running = false;
                            }
                            xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => self.exit = true,
                            _ => {}
                        }
                        self.visible = matches!(changed.state, xr::SessionState::VISIBLE | xr::SessionState::FOCUSED);
                        self.focused = changed.state == xr::SessionState::FOCUSED;
                    }
                    xr::StructureType::EVENT_DATA_INSTANCE_LOSS_PENDING => self.exit = true,
                    _ => {}
                }
                events(&e);
            }
        }
    }

    /// Asks the runtime to end the session (the player quits); it ends through the events.
    pub fn request_exit(&mut self, rt: &Runtime) {
        if self.running {
            // SAFETY: our running session
            unsafe { (rt.f.request_exit_session)(self.session) };
        } else {
            self.exit = true;
        }
    }

    /// Whether frames go to the headset: the session has begun.
    pub fn running(&self) -> bool {
        self.running
    }

    /// Waits for the runtime's next frame, begins it and finds where the eyes are at its display time.
    /// None while the session isn't running.
    pub fn begin_frame(&mut self, rt: &Runtime) -> Result<Option<Frame>, String> {
        if !self.running {
            return Ok(None);
        }
        // SAFETY: our running session, the frame calls in OpenXR's order
        unsafe {
            let wait = xr::FrameWaitInfo { ty: xr::StructureType::FRAME_WAIT_INFO, next: null() };
            let mut state = xr::FrameState { ty: xr::StructureType::FRAME_STATE, next: null_mut(), predicted_display_time: xr::Time::from_nanos(0), predicted_display_period: xr::Duration::from_nanos(0), should_render: xr::FALSE };
            let waited = SDL_GetTicksNS();
            check(rt, (rt.f.wait_frame)(self.session, &wait, &mut state), "waiting for the next frame")?;
            let waited_ns = SDL_GetTicksNS() - waited;
            let begin = xr::FrameBeginInfo { ty: xr::StructureType::FRAME_BEGIN_INFO, next: null() };
            check(rt, (rt.f.begin_frame)(self.session, &begin), "beginning a frame")?;
            let mut frame = Frame {
                display_time: state.predicted_display_time,
                period_ns: state.predicted_display_period.as_nanos(),
                should_render: state.should_render != xr::FALSE,
                waited_ns,
                views: [Pose { position: [0.0; 3], orientation: [0.0, 0.0, 0.0, 1.0] }; 2],
                fov: [Fov { left: -0.8, right: 0.8, up: 0.8, down: -0.8 }; 2],
                xr_views: [xr::View { ty: xr::StructureType::VIEW, next: null_mut(), pose: std::mem::zeroed(), fov: std::mem::zeroed() }; 2],
                located: false,
            };
            let locate = xr::ViewLocateInfo {
                ty: xr::StructureType::VIEW_LOCATE_INFO,
                next: null(),
                view_configuration_type: xr::ViewConfigurationType::PRIMARY_STEREO,
                display_time: state.predicted_display_time,
                space: self.space,
            };
            let mut view_state = xr::ViewState { ty: xr::StructureType::VIEW_STATE, next: null_mut(), view_state_flags: xr::ViewStateFlags::EMPTY };
            let mut n = 0;
            if (rt.f.locate_views)(self.session, &locate, &mut view_state, 2, &mut n, frame.xr_views.as_mut_ptr()) == xr::Result::SUCCESS
                && view_state.view_state_flags.contains(xr::ViewStateFlags::ORIENTATION_VALID | xr::ViewStateFlags::POSITION_VALID)
            {
                frame.located = true;
                for (eye, v) in frame.xr_views.iter().enumerate() {
                    frame.views[eye] = Pose::from_xr(&v.pose);
                    frame.fov[eye] = Fov { left: v.fov.angle_left, right: v.fov.angle_right, up: v.fov.angle_up, down: v.fov.angle_down };
                }
            }
            Ok(Some(frame))
        }
    }

    /// The image of `eye` to draw this frame into, acquired from its swapchain and waited for.
    pub fn eye_texture(&mut self, rt: &Runtime, eye: usize) -> Result<*mut SDL_GPUTexture, String> {
        let chain = &mut self.swapchains[eye];
        // SAFETY: our swapchain; an index the runtime hands out is one of its images
        unsafe {
            let mut index = 0;
            let acquire = xr::SwapchainImageAcquireInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_ACQUIRE_INFO, next: null() };
            check(rt, (rt.f.acquire_swapchain_image)(chain.handle, &acquire, &mut index), "acquiring an eye's image")?;
            chain.acquired = Some(index as usize);
            let wait = xr::SwapchainImageWaitInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_WAIT_INFO, next: null(), timeout: xr::Duration::INFINITE };
            check(rt, (rt.f.wait_swapchain_image)(chain.handle, &wait), "waiting for an eye's image")?;
            Ok(chain.textures[index as usize])
        }
    }

    /// Both eyes' images cleared to `colour` (linear), for frames with nothing else to draw (the
    /// library's): a projection layer under the panel, as compositors expect one. The size of each eye's
    /// image, all of it shown.
    pub fn clear_eyes(&mut self, rt: &Runtime, colour: SDL_FColor) -> Result<(u32, u32), String> {
        let targets = [self.eye_texture(rt, 0)?, self.eye_texture(rt, 1)?];
        // SAFETY: the eyes' images, acquired and waited for this frame, each cleared by a pass of its own
        unsafe {
            let command = SDL_AcquireGPUCommandBuffer(self.device);
            if command.is_null() {
                return Err(format!("clearing the eyes: {}", crate::render::sdl_error()));
            }
            for texture in targets {
                let mut info: SDL_GPUColorTargetInfo = std::mem::zeroed();
                info.texture = texture;
                info.clear_color = colour;
                info.load_op = SDL_GPU_LOADOP_CLEAR;
                info.store_op = SDL_GPU_STOREOP_STORE;
                let pass = SDL_BeginGPURenderPass(command, &info, 1, null());
                SDL_EndGPURenderPass(pass);
            }
            if !SDL_SubmitGPUCommandBuffer(command) {
                return Err(format!("clearing the eyes: {}", crate::render::sdl_error()));
            }
        }
        Ok(self.eye_size())
    }

    /// Ends the frame: the eyes' images given back (the GPU's work on them submitted before), and, when
    /// they were drawn, shown as a projection layer; the world blended where the picture's alpha is
    /// below 1 with alpha blend. `rect` is the part of each image drawn (dynamic resolution).
    ///
    /// `quads` go over the eyes' layer (or alone, with nothing drawn), in their order, each once it
    /// has a picture.
    pub fn end_frame(&mut self, rt: &Runtime, frame: &Frame, drawn: bool, rect: (u32, u32), quads: &[(Layer, Quad)]) -> Result<(), String> {
        // SAFETY: our running session, its swapchains' acquired images released once
        unsafe {
            for chain in &mut self.swapchains {
                if chain.acquired.take().is_some() {
                    let release = xr::SwapchainImageReleaseInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_RELEASE_INFO, next: null() };
                    check(rt, (rt.f.release_swapchain_image)(chain.handle, &release), "giving an eye's image back")?;
                }
            }
            let views: Vec<xr::CompositionLayerProjectionView> = (0..2)
                .map(|eye| xr::CompositionLayerProjectionView {
                    ty: xr::StructureType::COMPOSITION_LAYER_PROJECTION_VIEW,
                    next: null(),
                    pose: frame.xr_views[eye].pose,
                    fov: frame.xr_views[eye].fov,
                    sub_image: xr::SwapchainSubImage {
                        swapchain: self.swapchains[eye].handle,
                        image_rect: xr::Rect2Di { offset: xr::Offset2Di { x: 0, y: 0 }, extent: xr::Extent2Di { width: rect.0 as i32, height: rect.1 as i32 } },
                        image_array_index: 0,
                    },
                })
                .collect();
            let mut flags = xr::CompositionLayerFlags::CORRECT_CHROMATIC_ABERRATION;
            if rt.blend == xr::EnvironmentBlendMode::ALPHA_BLEND {
                flags |= xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA;
            }
            let layer = xr::CompositionLayerProjection {
                ty: xr::StructureType::COMPOSITION_LAYER_PROJECTION,
                next: null(),
                layer_flags: flags,
                space: self.space,
                view_count: 2,
                views: views.as_ptr(),
            };
            let shown = drawn && frame.should_render && frame.located;
            let mut layers: Vec<*const xr::CompositionLayerBaseHeader> = Vec::with_capacity(2);
            if shown {
                layers.push((&raw const layer).cast());
            }
            let quads: Vec<xr::CompositionLayerQuad> = quads
                .iter()
                .filter_map(|&(which, q)| {
                    let c = self.quad(which).filter(|c| c.shown && frame.should_render)?;
                    let chain = &c.chains[c.front];
                    // the pause menu's picture has straight alpha, its corners see through; the panel is whole
                    let layer_flags = match which {
                        Layer::Menu => xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA,
                        Layer::Panel => xr::CompositionLayerFlags::EMPTY,
                    };
                    Some(xr::CompositionLayerQuad {
                        ty: xr::StructureType::COMPOSITION_LAYER_QUAD,
                        next: null(),
                        layer_flags,
                        space: self.space,
                        eye_visibility: xr::EyeVisibility::BOTH,
                        sub_image: xr::SwapchainSubImage {
                            swapchain: chain.handle,
                            image_rect: xr::Rect2Di {
                                offset: xr::Offset2Di { x: 0, y: 0 },
                                extent: xr::Extent2Di { width: q.rect.0.min(chain.width) as i32, height: q.rect.1.min(chain.height) as i32 },
                            },
                            image_array_index: 0,
                        },
                        pose: q.pose.to_xr(),
                        size: xr::Extent2Df { width: q.size.0, height: q.size.1 },
                    })
                })
                .collect();
            layers.extend(quads.iter().map(|q| (q as *const xr::CompositionLayerQuad).cast::<xr::CompositionLayerBaseHeader>()));
            let end = xr::FrameEndInfo {
                ty: xr::StructureType::FRAME_END_INFO,
                next: null(),
                display_time: frame.display_time,
                environment_blend_mode: rt.blend,
                layer_count: layers.len() as u32,
                layers: if layers.is_empty() { null() } else { layers.as_ptr() },
            };
            check(rt, (rt.f.end_frame)(self.session, &end), "ending a frame")
        }
    }

    /// Ends the session the runtime's way (asked to end; empty frames handed over until it stops, 2 s at
    /// most), then releases its swapchains, space and session, before the GPU device and the runtime go.
    pub fn close(&mut self, rt: &Runtime) {
        if self.running {
            // SAFETY: our running session
            unsafe { (rt.f.request_exit_session)(self.session) };
            let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while self.running && std::time::Instant::now() < until {
                self.poll(rt, |_| {});
                if self.running
                    && let Ok(Some(frame)) = self.begin_frame(rt)
                {
                    let _ = self.end_frame(rt, &frame, false, (0, 0), &[]);
                }
            }
        }
        // the counters off before the session goes: SteamVR 2.17 kept them on with the instance and read the
        // ended session's timers from the next one, crashing it (the VPX fork's finding)
        if let Some(g) = self.gpu_time.take() {
            // SAFETY: our session
            unsafe {
                let state = xr::PerformanceMetricsStateMETA { ty: xr::StructureType::PERFORMANCE_METRICS_STATE_META, next: null(), enabled: xr::FALSE };
                (g.set_state)(self.session, &state);
            }
        }
        // SAFETY: each made here and released once
        unsafe {
            SDL_WaitForGPUIdle(self.device);
            for chain in self.swapchains.drain(..) {
                for t in chain.textures {
                    SDL_ReleaseGPUTexture(self.device, t);
                }
                (rt.f.destroy_swapchain)(chain.handle);
            }
            for quad in [self.panel.take(), self.menu.take()].into_iter().flatten() {
                self.release_quad(rt, quad);
            }
            if self.space != xr::Space::NULL {
                (rt.f.destroy_space)(self.space);
                self.space = xr::Space::NULL;
            }
            if self.session != xr::Session::NULL {
                if self.running {
                    (rt.f.end_session)(self.session);
                    self.running = false;
                }
                (rt.f.destroy_session)(self.session);
                self.session = xr::Session::NULL;
            }
        }
    }

    fn quad(&self, which: Layer) -> Option<&QuadChain> {
        match which {
            Layer::Panel => self.panel.as_ref(),
            Layer::Menu => self.menu.as_ref(),
        }
    }

    fn quad_slot(&mut self, which: Layer) -> &mut Option<QuadChain> {
        match which {
            Layer::Panel => &mut self.panel,
            Layer::Menu => &mut self.menu,
        }
    }

    /// # Safety
    /// On the thread that draws; the GPU done with the quad's images.
    unsafe fn release_quad(&self, rt: &Runtime, quad: QuadChain) {
        // SAFETY: as the caller promises
        unsafe {
            if !quad.writing.is_null() {
                SDL_ReleaseGPUFence(self.device, quad.writing);
            }
            for chain in quad.chains {
                self.drop_swapchain(rt, chain);
            }
            SDL_ReleaseGPUTransferBuffer(self.device, quad.transfer);
        }
    }

    /// # Safety
    /// On the thread that draws; the GPU done with the swapchain's images.
    unsafe fn drop_swapchain(&self, rt: &Runtime, chain: Swapchain) {
        // SAFETY: as the caller promises
        unsafe {
            for t in chain.textures {
                SDL_ReleaseGPUTexture(self.device, t);
            }
            (rt.f.destroy_swapchain)(chain.handle);
        }
    }

    /// Makes a quad's two swapchains, at least `width` by `height` pixels: made again for a bigger
    /// picture.
    ///
    /// # Safety
    /// On the thread that draws, while the session runs.
    pub unsafe fn make_quad(&mut self, rt: &Runtime, which: Layer, width: u32, height: u32) -> Result<(), String> {
        if self.quad(which).is_some_and(|q| q.chains[0].width >= width && q.chains[0].height >= height) {
            return Ok(());
        }
        let name = which.name();
        unsafe {
            if let Some(old) = self.quad_slot(which).take() {
                SDL_WaitForGPUIdle(self.device);
                self.release_quad(rt, old);
            }
            let mut count = 0;
            (rt.f.enumerate_swapchain_formats)(self.session, 0, &mut count, null_mut());
            let mut formats = vec![0i64; count as usize];
            (rt.f.enumerate_swapchain_formats)(self.session, count, &mut count, formats.as_mut_ptr());
            let Some(&(vk_format, format, bgra)) = QUAD_FORMATS.iter().find(|(f, ..)| formats.contains(f)) else {
                return Err(format!("the runtime offers no swapchain format for the {name} ({formats:?})"));
            };
            let first = self.quad_swapchain(rt, name, vk_format, format, width, height)?;
            let second = match self.quad_swapchain(rt, name, vk_format, format, width, height) {
                Ok(chain) => chain,
                Err(e) => {
                    self.drop_swapchain(rt, first);
                    return Err(e);
                }
            };
            let bi = SDL_GPUTransferBufferCreateInfo { usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD, size: width * height * 4, props: SDL_PropertiesID(0) };
            let transfer = SDL_CreateGPUTransferBuffer(self.device, &bi);
            if transfer.is_null() {
                self.drop_swapchain(rt, first);
                self.drop_swapchain(rt, second);
                return Err(format!("the {name}'s upload buffer: {}", crate::render::sdl_error()));
            }
            println!("VR: the {name}'s two swapchains, {} images each of {width}x{height}", first.textures.len());
            *self.quad_slot(which) = Some(QuadChain { chains: [first, second], front: 0, bgra, transfer, writing: null_mut(), shown: false });
            Ok(())
        }
    }

    /// One of a quad's swapchains, `width` by `height` pixels of `vk_format`, its images as SDL's
    /// textures of `format`.
    ///
    /// # Safety
    /// On the thread that draws, while the session runs.
    unsafe fn quad_swapchain(&self, rt: &Runtime, name: &str, vk_format: i64, format: SDL_GPUTextureFormat, width: u32, height: u32) -> Result<Swapchain, String> {
        unsafe {
            let info = xr::SwapchainCreateInfo {
                ty: xr::StructureType::SWAPCHAIN_CREATE_INFO,
                next: null(),
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                // its pictures uploaded into it
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT | xr::SwapchainUsageFlags::TRANSFER_DST | xr::SwapchainUsageFlags::SAMPLED,
                format: vk_format,
                sample_count: 1,
                width,
                height,
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            };
            let mut handle = xr::Swapchain::NULL;
            check(rt, (rt.f.create_swapchain)(self.session, &info, &mut handle), &format!("the {name}'s swapchain"))?;
            let mut n = 0;
            (rt.f.enumerate_swapchain_images)(handle, 0, &mut n, null_mut());
            let mut images = vec![xr::SwapchainImageVulkanKHR { ty: xr::StructureType::SWAPCHAIN_IMAGE_VULKAN_KHR, next: null_mut(), image: 0 }; n as usize];
            let r = (rt.f.enumerate_swapchain_images)(handle, n, &mut n, images.as_mut_ptr().cast());
            let mut chain = Swapchain { handle, width, height, textures: Vec::new(), acquired: None };
            let fail = |chain: Swapchain, e: String| {
                self.drop_swapchain(rt, chain);
                Err(e)
            };
            if r != xr::Result::SUCCESS {
                return fail(chain, format!("the {name}'s swapchain images: {}", rt.result(r)));
            }
            for image in &images {
                let props = SDL_CreateProperties();
                SDL_SetNumberProperty(props, TEXTURE_VULKAN_IMAGE.as_ptr(), image.image as i64);
                let mut ti: SDL_GPUTextureCreateInfo = std::mem::zeroed();
                ti.r#type = SDL_GPU_TEXTURETYPE_2D;
                ti.format = format;
                ti.usage = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET;
                ti.width = width;
                ti.height = height;
                ti.layer_count_or_depth = 1;
                ti.num_levels = 1;
                ti.sample_count = SDL_GPU_SAMPLECOUNT_1;
                ti.props = props;
                let texture = SDL_CreateGPUTexture(self.device, &ti);
                SDL_DestroyProperties(props);
                if texture.is_null() {
                    return fail(chain, format!("the {name}'s image as a texture: {}", crate::render::sdl_error()));
                }
                chain.textures.push(texture);
            }
            Ok(chain)
        }
    }

    /// A picture (`width` by `height`, rows packed, BGRA, else RGBA) uploaded by the GPU into the next
    /// image of the quad's swapchain not shown; handed over and shown once the GPU has written it: at
    /// once with `wait` (an earlier picture still being written finished first), else when
    /// `quad_settle` finds it done, the frame waiting for nothing. Without `wait`, nothing while an
    /// earlier picture is still being written (`quad_writing`). How long the runtime took to give the
    /// image.
    pub fn quad_picture(&mut self, rt: &Runtime, which: Layer, pixels: &[u8], width: u32, height: u32, bgra: bool, wait: bool) -> Result<Duration, String> {
        if wait && self.quad_writing(which) {
            self.quad_settle(rt, which, true)?;
        }
        let (device, name) = (self.device, which.name());
        let Some(quad) = self.quad_slot(which).as_mut() else { return Ok(Duration::ZERO) };
        let back = &quad.chains[1 - quad.front];
        let (handle, width, height) = (back.handle, width.min(back.width), height.min(back.height));
        let row = width as usize * 4;
        if !quad.writing.is_null() || width == 0 || height == 0 || pixels.len() < row * height as usize {
            return Ok(Duration::ZERO);
        }
        // SAFETY: our swapchain, an image acquired, waited for, written by the GPU's copy and released once
        // (by `quad_settle` once written, or here, unwritten, when the copy couldn't be sent); the upload
        // buffer is ours and holds `width * height * 4` bytes at most
        let waited = unsafe {
            let mut index = 0;
            let asked = Instant::now();
            let acquire = xr::SwapchainImageAcquireInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_ACQUIRE_INFO, next: null() };
            check(rt, (rt.f.acquire_swapchain_image)(handle, &acquire, &mut index), &format!("acquiring the {name}'s image"))?;
            let wait_info = xr::SwapchainImageWaitInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_WAIT_INFO, next: null(), timeout: xr::Duration::INFINITE };
            check(rt, (rt.f.wait_swapchain_image)(handle, &wait_info), &format!("waiting for the {name}'s image"))?;
            let waited = asked.elapsed();
            let mapped = SDL_MapGPUTransferBuffer(device, quad.transfer, true).cast::<u8>();
            let fence = if mapped.is_null() {
                null_mut()
            } else {
                let out = std::slice::from_raw_parts_mut(mapped, row * height as usize);
                out.copy_from_slice(&pixels[..row * height as usize]);
                if quad.bgra != bgra {
                    for px in out.chunks_exact_mut(4) {
                        px.swap(0, 2);
                    }
                }
                SDL_UnmapGPUTransferBuffer(device, quad.transfer);
                let cmd = SDL_AcquireGPUCommandBuffer(device);
                let pass = SDL_BeginGPUCopyPass(cmd);
                let from = SDL_GPUTextureTransferInfo { transfer_buffer: quad.transfer, offset: 0, pixels_per_row: width, rows_per_layer: height };
                let to = SDL_GPUTextureRegion { texture: quad.chains[1 - quad.front].textures[index as usize], mip_level: 0, layer: 0, x: 0, y: 0, z: 0, w: width, h: height, d: 1 };
                SDL_UploadToGPUTexture(pass, &from, &to, false);
                SDL_EndGPUCopyPass(pass);
                // handed over only once written: the runtime may read it on a device of its own, which
                // waits for nothing of ours
                SDL_SubmitGPUCommandBufferAndAcquireFence(cmd)
            };
            if fence.is_null() {
                // nothing written: handed back as it was, the layer still showing the other swapchain
                let release = xr::SwapchainImageReleaseInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_RELEASE_INFO, next: null() };
                check(rt, (rt.f.release_swapchain_image)(handle, &release), &format!("handing the {name}'s image back"))?;
                return Err(format!("the {name}'s picture: {}", crate::render::sdl_error()));
            }
            quad.writing = fence;
            waited
        };
        if wait {
            self.quad_settle(rt, which, true)?;
        }
        Ok(waited)
    }

    /// Hands over the picture the GPU has written into the quad's swapchain not shown (`quad_picture`),
    /// once its copy is done, or with `wait` once it is: the layer shows that swapchain from then on.
    /// Whether it did.
    pub fn quad_settle(&mut self, rt: &Runtime, which: Layer, wait: bool) -> Result<bool, String> {
        let (device, name) = (self.device, which.name());
        let Some(quad) = self.quad_slot(which).as_mut().filter(|q| !q.writing.is_null()) else { return Ok(false) };
        // SAFETY: the fence of our copy, released once; the image it wrote acquired from that swapchain
        unsafe {
            let done = if wait { SDL_WaitForGPUFences(device, true, &quad.writing, 1) } else { SDL_QueryGPUFence(device, quad.writing) };
            if !done {
                return if wait { Err(format!("the {name}'s picture: {}", crate::render::sdl_error())) } else { Ok(false) };
            }
            SDL_ReleaseGPUFence(device, quad.writing);
            quad.writing = null_mut();
            let back = 1 - quad.front;
            let release = xr::SwapchainImageReleaseInfo { ty: xr::StructureType::SWAPCHAIN_IMAGE_RELEASE_INFO, next: null() };
            check(rt, (rt.f.release_swapchain_image)(quad.chains[back].handle, &release), &format!("handing the {name}'s image over"))?;
            quad.front = back;
            quad.shown = true;
        }
        Ok(true)
    }

    /// The GPU is still writing a picture of the quad's (`quad_picture`, not yet `quad_settle`d).
    pub fn quad_writing(&self, which: Layer) -> bool {
        self.quad(which).is_some_and(|q| !q.writing.is_null())
    }

    /// The session's handle and space, for the controls.
    pub fn handles(&self) -> (xr::Session, xr::Space) {
        (self.session, self.space)
    }

    /// Turns the headset's eye tracker on for this session: an eye-tracked foveation profile at level
    /// NONE applied to both eyes' swapchains, which asks the runtime to do nothing with our images (the
    /// fork's way, its §11); the runtime's foveation centres can be read then.
    unsafe fn track_eyes(&self, rt: &Runtime) -> Option<xr::pfn::GetFoveationEyeTrackedStateMETA> {
        if !rt.eye_tracked_foveation || !rt.has(FB_FOVEATION) || !rt.has(FB_FOVEATION_CONFIGURATION) || !rt.has(FB_SWAPCHAIN_UPDATE_STATE) {
            return None;
        }
        // SAFETY: the extensions' functions, with their structures chained as their specification says
        unsafe {
            let create: xr::pfn::CreateFoveationProfileFB = function(rt.get, rt.instance, "xrCreateFoveationProfileFB").ok()?;
            let destroy: xr::pfn::DestroyFoveationProfileFB = function(rt.get, rt.instance, "xrDestroyFoveationProfileFB").ok()?;
            let update: xr::pfn::UpdateSwapchainFB = function(rt.get, rt.instance, "xrUpdateSwapchainFB").ok()?;
            let state: xr::pfn::GetFoveationEyeTrackedStateMETA = function(rt.get, rt.instance, "xrGetFoveationEyeTrackedStateMETA").ok()?;
            let tracked = xr::FoveationEyeTrackedProfileCreateInfoMETA {
                ty: xr::StructureType::FOVEATION_EYE_TRACKED_PROFILE_CREATE_INFO_META,
                next: null(),
                flags: xr::FoveationEyeTrackedProfileCreateFlagsMETA::EMPTY,
            };
            let level = xr::FoveationLevelProfileCreateInfoFB {
                ty: xr::StructureType::FOVEATION_LEVEL_PROFILE_CREATE_INFO_FB,
                next: (&raw const tracked).cast_mut().cast(),
                level: xr::FoveationLevelFB::NONE,
                vertical_offset: 0.0,
                dynamic: xr::FoveationDynamicFB::DISABLED,
            };
            let info = xr::FoveationProfileCreateInfoFB { ty: xr::StructureType::FOVEATION_PROFILE_CREATE_INFO_FB, next: (&raw const level).cast_mut().cast() };
            let mut profile = xr::FoveationProfileFB::NULL;
            let r = create(self.session, &info, &mut profile);
            if r != xr::Result::SUCCESS {
                println!("VR foveation: no eye-tracked profile ({})", rt.result(r));
                return None;
            }
            let mut applied = true;
            for chain in &self.swapchains {
                let st = xr::SwapchainStateFoveationFB {
                    ty: xr::StructureType::SWAPCHAIN_STATE_FOVEATION_FB,
                    next: null_mut(),
                    flags: xr::SwapchainStateFoveationFlagsFB::EMPTY,
                    profile,
                };
                let r = update(chain.handle, (&raw const st).cast());
                if r != xr::Result::SUCCESS {
                    println!("VR foveation: the eye-tracked profile wasn't applied ({})", rt.result(r));
                    applied = false;
                }
            }
            destroy(profile);
            applied.then_some(state)
        }
    }

    /// The runtime's foveation centres, each eye's, -1 to 1 (x right, y down, as the fork found them),
    /// while the eye tracker gives them.
    pub fn foveation_centres(&self) -> Option<[[f32; 2]; 2]> {
        let get = self.eye_tracked_state?;
        // SAFETY: the extension's function, on our session
        unsafe {
            let mut s: xr::FoveationEyeTrackedStateMETA = std::mem::zeroed();
            s.ty = xr::StructureType::FOVEATION_EYE_TRACKED_STATE_META;
            (get(self.session, &mut s) == xr::Result::SUCCESS && s.flags.contains(xr::FoveationEyeTrackedStateFlagsMETA::VALID))
                .then(|| [0, 1].map(|e| [s.foveation_center[e].x, s.foveation_center[e].y]))
        }
    }
}

/// The runtime's GPU time counter for the app's frames.
struct GpuTime {
    query: xr::pfn::QueryPerformanceMetricsCounterMETA,
    set_state: xr::pfn::SetPerformanceMetricsStateMETA,
    path: xr::Path,
}

impl GpuTime {
    /// The counter, switched on, when the runtime has it.
    fn enable(rt: &Runtime, session: xr::Session) -> Option<GpuTime> {
        if !rt.has(PERFORMANCE_METRICS) {
            return None;
        }
        // SAFETY: the extension's functions, with arrays of the counts given
        unsafe {
            let get = rt.get;
            let enumerate: xr::pfn::EnumeratePerformanceMetricsCounterPathsMETA = function(get, rt.instance, "xrEnumeratePerformanceMetricsCounterPathsMETA").ok()?;
            let set_state: xr::pfn::SetPerformanceMetricsStateMETA = function(get, rt.instance, "xrSetPerformanceMetricsStateMETA").ok()?;
            let query: xr::pfn::QueryPerformanceMetricsCounterMETA = function(get, rt.instance, "xrQueryPerformanceMetricsCounterMETA").ok()?;
            let mut n = 0;
            enumerate(rt.instance, 0, &mut n, null_mut());
            let mut paths = vec![xr::Path::NULL; n as usize];
            enumerate(rt.instance, n, &mut n, paths.as_mut_ptr());
            let path = paths.into_iter().find(|&p| {
                let mut buf = [0 as c_char; 256];
                let mut len = 0;
                (rt.f.path_to_string)(rt.instance, p, buf.len() as u32, &mut len, buf.as_mut_ptr()) == xr::Result::SUCCESS
                    && CStr::from_ptr(buf.as_ptr()).to_bytes() == b"/perfmetrics_meta/app/gpu_frametime"
            })?;
            let state = xr::PerformanceMetricsStateMETA { ty: xr::StructureType::PERFORMANCE_METRICS_STATE_META, next: null(), enabled: xr::TRUE };
            (set_state(session, &state) == xr::Result::SUCCESS).then_some(GpuTime { query, set_state, path })
        }
    }
}

/// A frame the runtime asked for: when it shows, and where the eyes are then.
pub struct Frame {
    pub display_time: xr::Time,
    pub period_ns: i64,
    /// The runtime wants this frame drawn (false while the headset isn't worn, on SteamVR).
    pub should_render: bool,
    /// How long `xrWaitFrame` held the player.
    pub waited_ns: u64,
    pub views: [Pose; 2],
    pub fov: [Fov; 2],
    xr_views: [xr::View; 2],
    /// The eyes were found (their poses are valid).
    pub located: bool,
}

impl Frame {
    /// The eyes pinned where a player stands before a cabinet, for test runs (`VPI_XR_PIN_EYES`): 25 cm
    /// behind and 50 cm above the playfield's front, 64 mm apart, looking down a little, as the VPX
    /// fork's performance runs pin them, in a room whose floor is at 0.
    pub fn pin_eyes(&mut self) {
        // test runs may look elsewhere: further down (`VPI_XR_PIN_PITCH=70`, degrees below level) or from
        // elsewhere (`VPI_XR_PIN_AT=0,1.2,0.4`, metres, the eyes' middle)
        let pitch = std::env::var("VPI_XR_PIN_PITCH").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(20.0);
        let at = std::env::var("VPI_XR_PIN_AT")
            .ok()
            .and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                (p.len() == 3).then(|| [p[0], p[1], p[2]])
            })
            .unwrap_or([0.0, 1.35, 0.25]);
        let tilt = (-pitch).to_radians() * 0.5;
        let q = [tilt.sin(), 0.0, 0.0, tilt.cos()];
        for (eye, x) in [-0.032f32, 0.032].into_iter().enumerate() {
            let (x, y, z) = (at[0] + x, at[1], at[2]);
            self.views[eye] = Pose { position: [x, y, z], orientation: q };
            self.xr_views[eye].pose =
                xr::Posef { orientation: xr::Quaternionf { x: q[0], y: q[1], z: q[2], w: q[3] }, position: xr::Vector3f { x, y, z } };
            if !self.located {
                let f = Fov { left: -0.85, right: 0.75, up: 0.85, down: -0.9 };
                self.fov[eye] = if eye == 0 { f } else { Fov { left: -f.right, right: -f.left, ..f } };
                self.xr_views[eye].fov = xr::Fovf { angle_left: self.fov[eye].left, angle_right: self.fov[eye].right, angle_up: self.fov[eye].up, angle_down: self.fov[eye].down };
            }
        }
        self.located = true;
        self.should_render = true;
    }
}
