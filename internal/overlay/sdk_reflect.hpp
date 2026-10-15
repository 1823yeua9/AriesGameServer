// Reflection-based SDK helpers for the overlay's dev actions.
//
// Design principle (per project directive): all dev-menu actions use UE's own
// name-based reflection — StaticFindObject style lookups through GObjects, path
// resolution via StaticLoadObject, UFunction dispatch via ProcessEvent — NOT
// hardcoded byte offsets. Rationale: offsets break every patch; reflection
// identities (class names, function names, package paths) are stable across the
// whole product line. The dumper7 SDK is used for TYPE DEFINITIONS (so we can
// pack ProcessEvent parameter blocks correctly) but field-offset constants are
// avoided except where a UPROPERTY's annotated offset in the generated header
// is itself the canonical identity.
//
// GObjects lookups intentionally do not cache UObject pointers across world
// travel or garbage collection.

#pragma once
#include <string>

// Forward-declare the SDK types we touch so this header stays lean.
namespace SDK {
    class UObject;
    class UClass;
    class UFunction;
    class UEngine;
    class UWorld;
} // namespace SDK

namespace sdk_reflect {
    // Guard that GObjects is initialized (via valorant_sdk_adapter) and the
    // array walk is safe. Returns false if the live process state is not yet
    // ready (eg. a session is transitioning worlds).
    bool ensure_ready();

    // ---- object lookups ----
    // Find a UObject by name. If `owner_name` is nullptr, matches ANY object
    // with that short name. If `owner_name` is set, further restricts to those
    // whose `Outer->Name` matches. Both match the exact `Name` field (not path).
    SDK::UObject* find_object(const char* name, const char* owner_name = nullptr);

    // Same as find_object but requires the result to be a UClass instance.
    SDK::UClass* find_class(const char* name);

    // Same as find_object but requires the result to be a UFunction and lives
    // on the class whose Name matches `owner_class`.
    SDK::UFunction* find_function(const char* owner_class, const char* function_name);

    // Resolve a package-qualified path like "/Game/Characters/Wushu/Wushu_PC"
    // to its loaded UObject (StaticLoadObject-style). Returns null if the
    // asset isn't currently loaded — this helper does NOT trigger a load.
    SDK::UObject* find_by_path(const char* package_path);

    // Read an object-pointer UPROPERTY by its reflected name. Returns null
    // when absent or unreadable; never writes to the object.
    SDK::UObject* object_property(const SDK::UObject* object, const char* property_name);

    // Currently resolves already-loaded assets only. Explicit async loading
    // requires a game-thread dispatcher and is not implemented.
    SDK::UObject* load_object(const char* package_path);

    // ---- shortcuts to the live engine/world/player ----
    SDK::UEngine* engine();
    SDK::UWorld* world();
    SDK::UObject* local_player_controller(); // first LocalPlayer's controller
    SDK::UObject* local_player_pawn();       // its possessed pawn, if any

    // ---- generic ProcessEvent invoke ----
    // Looks up `UClass::function_name` and ProcessEvents it on `target` with
    // the supplied parameter block. Returns true if the UFunction was found
    // and ProcessEvent returned. Caller is responsible for
    // laying out `params` to match the SDK Params:: struct for that function.
    bool invoke(SDK::UObject* target, const char* owner_class, const char* function_name, void* params);

    // Convenience: invoke a static Default__ class function (eg. KismetStringLibrary,
    // GameplayStatics). Looks up Default__<owner_class> as the ProcessEvent target.
    bool invoke_static(const char* owner_class, const char* function_name, void* params);

    // ---- quick status line for the overlay UI ----
    // Returns a short human-readable "last action" message updated by the
    // reflection layer so the overlay can show feedback without plumbing
    // string returns through every action.
    const char* last_status();
    void set_last_status(const char* fmt, ...);
} // namespace sdk_reflect
