// An OpenXR API layer for test runs on the Steam Frame (versus.py), so that VPI and the owner's VPX fork can be
// measured the same way with the headset unworn. Unworn, SteamVR says not to draw (shouldRender false) and the eyes
// are wherever the headset lies.
//
//   VPI_XR_LAYER_PIN=1          says draw, and puts both eyes where a player stands before the cabinet: VPI's own pin
//                               (25 cm behind and 50 cm above the lockbar, 64 mm apart, looking 20 degrees down, in
//                               the floor-level space), so that either player draws the same view; the head too
//   VPI_XR_LAYER_LOG=<file>     a line for each frame: when it was waited for, begun and ended, the display time it
//                               was for, the runtime's count of the application's GPU time, and the head it was drawn
//                               for (where it looked, in degrees, and the eyes' middle in the space it was drawn in)
//
// Enabled for one process by XR_API_LAYER_PATH=<this folder> XR_ENABLE_API_LAYERS=XR_APILAYER_VPI_test. Ours.

#include <math.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include <openxr/openxr.h>
#include <openxr/openxr_loader_negotiation.h>

#define EXPORT __attribute__((visibility("default")))

static PFN_xrGetInstanceProcAddr next_gipa;
static PFN_xrCreateSession next_xrCreateSession;
static PFN_xrCreateReferenceSpace next_xrCreateReferenceSpace;
static PFN_xrDestroySpace next_xrDestroySpace;
static PFN_xrWaitFrame next_xrWaitFrame;
static PFN_xrBeginFrame next_xrBeginFrame;
static PFN_xrEndFrame next_xrEndFrame;
static PFN_xrLocateViews next_xrLocateViews;
static PFN_xrLocateSpace next_xrLocateSpace;
static PFN_xrStringToPath next_xrStringToPath;
static PFN_xrQueryPerformanceMetricsCounterMETA next_xrQueryPerformanceMetricsCounterMETA;

static XrInstance g_instance;
static XrSession g_session;
static int g_pin;
static FILE* g_log;
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;

// The app's own reference spaces of the kinds that matter here, and ours (the floor-level space, to pin in another)
#define MAX_SPACES 32
static XrSpace g_view_spaces[MAX_SPACES], g_stage_spaces[MAX_SPACES];
static XrSpace g_our_stage;

static const float EYE_HEIGHT = 1.35f, EYE_BACK = 0.25f, EYE_HALF_IPD = 0.032f, TILT_DEGREES = -20.0f;

// Frames waited for and not yet ended, matched to xrEndFrame by their display time
#define RING 16
typedef struct {
    XrTime display;
    XrDuration period;
    double wait_call, wait_return, begin;
    int should_render, used;
} Waited;
static Waited g_ring[RING];
static unsigned g_waits, g_frames;
static XrTime g_display0;
static XrPath g_gpu_path;

static double now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static int has(XrSpace* set, XrSpace s) {
    for (int i = 0; i < MAX_SPACES; i++)
        if (set[i] == s && s != XR_NULL_HANDLE) return 1;
    return 0;
}

static void add(XrSpace* set, XrSpace s) {
    for (int i = 0; i < MAX_SPACES; i++)
        if (set[i] == XR_NULL_HANDLE) { set[i] = s; return; }
}

static void drop(XrSpace* set, XrSpace s) {
    for (int i = 0; i < MAX_SPACES; i++)
        if (set[i] == s) set[i] = XR_NULL_HANDLE;
}

static XrQuaternionf mul(XrQuaternionf a, XrQuaternionf b) {
    XrQuaternionf q = {
        a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
        a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
        a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
    };
    return q;
}

static XrVector3f rotate(XrQuaternionf q, XrVector3f v) {
    XrQuaternionf p = {v.x, v.y, v.z, 0.0f}, c = {-q.x, -q.y, -q.z, q.w};
    XrQuaternionf r = mul(mul(q, p), c);
    XrVector3f out = {r.x, r.y, r.z};
    return out;
}

// b expressed in a's parent: a * b
static XrPosef compose(XrPosef a, XrPosef b) {
    XrVector3f p = rotate(a.orientation, b.position);
    XrPosef out = {mul(a.orientation, b.orientation), {a.position.x + p.x, a.position.y + p.y, a.position.z + p.z}};
    return out;
}

// The pinned pose of a point between the eyes (x: its offset to the right) in `space` at `time`.
static int pinned(XrSpace space, XrTime time, float x, XrPosef* out) {
    const float half = TILT_DEGREES * 0.5f * (float)M_PI / 180.0f;
    XrPosef stage = {{sinf(half), 0.0f, 0.0f, cosf(half)}, {x, EYE_HEIGHT, EYE_BACK}};
    if (has(g_stage_spaces, space)) {
        *out = stage;
        return 1;
    }
    if (g_our_stage == XR_NULL_HANDLE) {
        XrReferenceSpaceCreateInfo info = {.type = XR_TYPE_REFERENCE_SPACE_CREATE_INFO};
        info.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_STAGE;
        info.poseInReferenceSpace.orientation.w = 1.0f;
        if (XR_FAILED(next_xrCreateReferenceSpace(g_session, &info, &g_our_stage))) return 0;
    }
    XrSpaceLocation origin = {.type = XR_TYPE_SPACE_LOCATION};
    if (XR_FAILED(next_xrLocateSpace(g_our_stage, space, time, &origin))) return 0;
    const XrSpaceLocationFlags valid = XR_SPACE_LOCATION_ORIENTATION_VALID_BIT | XR_SPACE_LOCATION_POSITION_VALID_BIT;
    if ((origin.locationFlags & valid) != valid) return 0;
    *out = compose(origin.pose, stage);
    return 1;
}

static XrResult XRAPI_CALL layer_xrCreateSession(XrInstance instance, const XrSessionCreateInfo* info, XrSession* session) {
    XrResult r = next_xrCreateSession(instance, info, session);
    if (XR_SUCCEEDED(r)) g_session = *session;
    return r;
}

static XrResult XRAPI_CALL layer_xrCreateReferenceSpace(XrSession session, const XrReferenceSpaceCreateInfo* info, XrSpace* space) {
    XrResult r = next_xrCreateReferenceSpace(session, info, space);
    if (XR_SUCCEEDED(r)) {
        pthread_mutex_lock(&g_lock);
        // a space offset from the room's floor isn't the floor-level space itself: pinned through ours
        const XrPosef p = info->poseInReferenceSpace;
        const int identity = p.position.x == 0.0f && p.position.y == 0.0f && p.position.z == 0.0f && p.orientation.w == 1.0f;
        if (info->referenceSpaceType == XR_REFERENCE_SPACE_TYPE_VIEW) add(g_view_spaces, *space);
        if (info->referenceSpaceType == XR_REFERENCE_SPACE_TYPE_STAGE && identity) add(g_stage_spaces, *space);
        pthread_mutex_unlock(&g_lock);
    }
    return r;
}

static XrResult XRAPI_CALL layer_xrDestroySpace(XrSpace space) {
    pthread_mutex_lock(&g_lock);
    drop(g_view_spaces, space);
    drop(g_stage_spaces, space);
    pthread_mutex_unlock(&g_lock);
    return next_xrDestroySpace(space);
}

static XrResult XRAPI_CALL layer_xrWaitFrame(XrSession session, const XrFrameWaitInfo* info, XrFrameState* state) {
    const double called = now_ms();
    XrResult r = next_xrWaitFrame(session, info, state);
    const double returned = now_ms();
    if (XR_SUCCEEDED(r)) {
        pthread_mutex_lock(&g_lock);
        Waited* w = &g_ring[g_waits++ % RING];
        *w = (Waited){state->predictedDisplayTime, state->predictedDisplayPeriod, called, returned, -1.0, state->shouldRender, 1};
        pthread_mutex_unlock(&g_lock);
        if (g_pin) state->shouldRender = XR_TRUE;
    }
    return r;
}

static XrResult XRAPI_CALL layer_xrBeginFrame(XrSession session, const XrFrameBeginInfo* info) {
    const double t = now_ms();
    XrResult r = next_xrBeginFrame(session, info);
    pthread_mutex_lock(&g_lock);
    // the oldest frame waited for and not begun
    Waited* first = NULL;
    for (int i = 0; i < RING; i++)
        if (g_ring[i].used && g_ring[i].begin < 0.0 && (first == NULL || g_ring[i].display < first->display)) first = &g_ring[i];
    if (first) first->begin = t;
    pthread_mutex_unlock(&g_lock);
    return r;
}

static XrResult XRAPI_CALL layer_xrEndFrame(XrSession session, const XrFrameEndInfo* info) {
    const double called = now_ms();
    XrResult r = next_xrEndFrame(session, info);
    const double returned = now_ms();
    if (g_log == NULL) return r;
    float gpu = -1.0f;
    if (next_xrQueryPerformanceMetricsCounterMETA && g_gpu_path != XR_NULL_PATH) {
        XrPerformanceMetricsCounterMETA c = {.type = XR_TYPE_PERFORMANCE_METRICS_COUNTER_META};
        if (XR_SUCCEEDED(next_xrQueryPerformanceMetricsCounterMETA(session, g_gpu_path, &c)) &&
            (c.counterFlags & XR_PERFORMANCE_METRICS_COUNTER_FLOAT_VALUE_VALID_BIT_META))
            gpu = c.floatValue;
    }
    // the head the frame was drawn for: the first projection layer's eyes, where the left one looks (pitch up
    // positive, yaw left positive) and their middle, in the layer's space
    float pitch = NAN, yaw = NAN;
    XrVector3f head = {NAN, NAN, NAN};
    for (uint32_t i = 0; i < info->layerCount; i++) {
        const XrCompositionLayerProjection* p = (const XrCompositionLayerProjection*)info->layers[i];
        if (p == NULL || p->type != XR_TYPE_COMPOSITION_LAYER_PROJECTION || p->viewCount < 2) continue;
        const XrPosef a = p->views[0].pose, b = p->views[1].pose;
        const XrVector3f ahead = rotate(a.orientation, (XrVector3f){0.0f, 0.0f, -1.0f});
        pitch = asinf(fmaxf(-1.0f, fminf(1.0f, ahead.y))) * 180.0f / (float)M_PI;
        yaw = atan2f(-ahead.x, -ahead.z) * 180.0f / (float)M_PI;
        head = (XrVector3f){(a.position.x + b.position.x) * 0.5f, (a.position.y + b.position.y) * 0.5f,
                            (a.position.z + b.position.z) * 0.5f};
        break;
    }
    pthread_mutex_lock(&g_lock);
    Waited* w = NULL;
    for (int i = 0; i < RING; i++)
        if (g_ring[i].used && g_ring[i].display == info->displayTime) w = &g_ring[i];
    if (g_frames == 0) {
        g_display0 = info->displayTime;
        // times on CLOCK_MONOTONIC (Python's time.monotonic()), in ms; the display time from the first frame's
        fprintf(g_log, "frame,wait_ms,waited_ms,begin_ms,end_ms,end_call_ms,display_ms,period_ms,should_render,layers,gpu_ms,"
                       "pitch_deg,yaw_deg,head_x,head_y,head_z\n");
    }
    fprintf(g_log, "%u,%.3f,%.3f,%.3f,%.3f,%.3f,%.3f,%.3f,%d,%u,%.3f,%.1f,%.1f,%.3f,%.3f,%.3f\n", g_frames++,
            w ? w->wait_call : -1.0, w ? w->wait_return - w->wait_call : -1.0, w && w->begin >= 0.0 ? w->begin : -1.0,
            returned, returned - called, (info->displayTime - g_display0) / 1e6, w ? w->period / 1e6 : -1.0,
            w ? w->should_render : -1, info->layerCount, gpu, pitch, yaw, head.x, head.y, head.z);
    if (w) w->used = 0;
    if (g_frames % 72 == 0) fflush(g_log);
    pthread_mutex_unlock(&g_lock);
    return r;
}

static XrResult XRAPI_CALL layer_xrLocateViews(XrSession session, const XrViewLocateInfo* info, XrViewState* state,
                                                uint32_t capacity, uint32_t* count, XrView* views) {
    XrResult r = next_xrLocateViews(session, info, state, capacity, count, views);
    if (!g_pin || XR_FAILED(r) || views == NULL || capacity < 2 || *count < 2) return r;
    for (uint32_t i = 0; i < 2; i++) {
        XrPosef p;
        if (!pinned(info->space, info->displayTime, i == 0 ? -EYE_HALF_IPD : EYE_HALF_IPD, &p)) return r;
        views[i].pose = p;
        if (views[i].fov.angleLeft == 0.0f && views[i].fov.angleRight == 0.0f) {
            // the Frame's own (VPI's pin uses the same), the right eye's mirrored
            views[i].fov = i == 0 ? (XrFovf){-0.85f, 0.75f, 0.85f, -0.9f} : (XrFovf){-0.75f, 0.85f, 0.85f, -0.9f};
        }
    }
    state->viewStateFlags = XR_VIEW_STATE_ORIENTATION_VALID_BIT | XR_VIEW_STATE_POSITION_VALID_BIT |
                            XR_VIEW_STATE_ORIENTATION_TRACKED_BIT | XR_VIEW_STATE_POSITION_TRACKED_BIT;
    return r;
}

static XrResult XRAPI_CALL layer_xrLocateSpace(XrSpace space, XrSpace base, XrTime time, XrSpaceLocation* location) {
    XrResult r = next_xrLocateSpace(space, base, time, location);
    if (!g_pin || XR_FAILED(r)) return r;
    pthread_mutex_lock(&g_lock);
    const int head = has(g_view_spaces, space);
    pthread_mutex_unlock(&g_lock);
    XrPosef p;
    if (head && pinned(base, time, 0.0f, &p)) {
        location->pose = p;
        location->locationFlags = XR_SPACE_LOCATION_ORIENTATION_VALID_BIT | XR_SPACE_LOCATION_POSITION_VALID_BIT |
                                  XR_SPACE_LOCATION_ORIENTATION_TRACKED_BIT | XR_SPACE_LOCATION_POSITION_TRACKED_BIT;
    }
    return r;
}

static XrResult XRAPI_CALL layer_xrGetInstanceProcAddr(XrInstance instance, const char* name, PFN_xrVoidFunction* function);

#define INTERCEPT(f)                                    \
    if (strcmp(name, #f) == 0) {                        \
        *function = (PFN_xrVoidFunction)layer_##f;      \
        return XR_SUCCESS;                              \
    }

static XrResult XRAPI_CALL layer_xrGetInstanceProcAddr(XrInstance instance, const char* name, PFN_xrVoidFunction* function) {
    INTERCEPT(xrGetInstanceProcAddr)
    INTERCEPT(xrCreateSession)
    INTERCEPT(xrCreateReferenceSpace)
    INTERCEPT(xrDestroySpace)
    INTERCEPT(xrWaitFrame)
    INTERCEPT(xrBeginFrame)
    INTERCEPT(xrEndFrame)
    INTERCEPT(xrLocateViews)
    INTERCEPT(xrLocateSpace)
    return next_gipa ? next_gipa(instance, name, function) : XR_ERROR_HANDLE_INVALID;
}

static XrResult XRAPI_CALL layer_xrCreateApiLayerInstance(const XrInstanceCreateInfo* info, const XrApiLayerCreateInfo* layer,
                                                          XrInstance* instance) {
    XrApiLayerNextInfo* next = layer->nextInfo;
    XrApiLayerCreateInfo next_layer = *layer;
    next_layer.nextInfo = next->next;
    XrResult r = next->nextCreateApiLayerInstance(info, &next_layer, instance);
    if (XR_FAILED(r)) return r;
    next_gipa = next->nextGetInstanceProcAddr;
    g_instance = *instance;
#define NEXT(f) next_gipa(*instance, #f, (PFN_xrVoidFunction*)&next_##f)
    NEXT(xrCreateSession);
    NEXT(xrCreateReferenceSpace);
    NEXT(xrDestroySpace);
    NEXT(xrWaitFrame);
    NEXT(xrBeginFrame);
    NEXT(xrEndFrame);
    NEXT(xrLocateViews);
    NEXT(xrLocateSpace);
    NEXT(xrStringToPath);
    // there only when the application enabled XR_META_performance_metrics
    if (XR_FAILED(NEXT(xrQueryPerformanceMetricsCounterMETA))) next_xrQueryPerformanceMetricsCounterMETA = NULL;
#undef NEXT
    if (next_xrStringToPath) next_xrStringToPath(*instance, "/perfmetrics_meta/app/gpu_frametime", &g_gpu_path);
    const char* pin = getenv("VPI_XR_LAYER_PIN");
    g_pin = pin != NULL && strcmp(pin, "1") == 0;
    const char* log = getenv("VPI_XR_LAYER_LOG");
    if (log != NULL && *log != '\0' && g_log == NULL) g_log = fopen(log, "w");
    return r;
}

EXPORT XrResult XRAPI_CALL xrNegotiateLoaderApiLayerInterface(const XrNegotiateLoaderInfo* loader, const char* name,
                                                             XrNegotiateApiLayerRequest* request) {
    (void)name;
    if (loader == NULL || request == NULL || loader->structType != XR_LOADER_INTERFACE_STRUCT_LOADER_INFO ||
        request->structType != XR_LOADER_INTERFACE_STRUCT_API_LAYER_REQUEST ||
        loader->minInterfaceVersion > XR_CURRENT_LOADER_API_LAYER_VERSION ||
        loader->maxInterfaceVersion < XR_CURRENT_LOADER_API_LAYER_VERSION)
        return XR_ERROR_INITIALIZATION_FAILED;
    request->layerInterfaceVersion = XR_CURRENT_LOADER_API_LAYER_VERSION;
    request->layerApiVersion = XR_CURRENT_API_VERSION > loader->maxApiVersion ? loader->maxApiVersion : XR_CURRENT_API_VERSION;
    request->getInstanceProcAddr = layer_xrGetInstanceProcAddr;
    request->createApiLayerInstance = layer_xrCreateApiLayerInstance;
    return XR_SUCCESS;
}
