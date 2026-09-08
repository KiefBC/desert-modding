# Changes against hudhook 0.9.2

This is the published [hudhook](https://github.com/veeenu/hudhook) 0.9.2 crate (MIT) with the
changes below applied. It is patched into the workspace by `[patch.crates-io]` in the root
`Cargo.toml`; nothing else in the tree depends on it. Everything here is meant to be upstreamable
as it stands, and to be re-applicable to a later hudhook by hand: items 1 to 6 are six small hunks
in one file, item 7 is a two-line change in a second, and item 8 adds one file and touches three.

## Why

Crimson Desert (Steam build 25116796) would not launch with `DesertOverlay.asi` installed. The
sequence, from `ReShade.log` and `DesertOverlay.log`:

1. The game creates its swapchain with `IDXGIFactory2::CreateSwapChainForHwnd`
   (3840x2160, `R10G10B10A2_UNORM`, flip discard, flags `0x800`) about six seconds in.
2. Five seconds later it calls `IDXGISwapChain::ResizeBuffers(0, 0, 0, 0, 0)`, which fails with
   `E_INVALIDARG`.
3. It falls back: `SetFullscreenState(FALSE)`, a new command queue, then
   `CreateSwapChainForHwnd` for the *same* HWND twice. Both fail with `0x80070005`
   `E_ACCESSDENIED` - DXGI refuses to give a window a second swapchain while its previous one is
   still alive.
4. The game exits before the main menu. No Windows crash event, no hudhook WARN or ERROR.

Upstream 0.9.2 is what kept that swapchain alive. Every `Present` calls
`INIT_STATE.lock().insert_swap_chain(swap_chain3)`, which stores a **strong** `IDXGISwapChain3`
clone in the `WithSwapChain` / `Complete` states. Nothing releases it before a DXGI call that
validates the reference count:

- `ResizeBuffers`, `ResizeBuffers1` and `SetSourceSize` only `wait_for_pipeline_idle_before` and
  then call the trampoline;
- `handle_swap_chain_created` and every `reset_pipeline` in the create-swapchain hooks run
  *after* the trampoline has returned, so the stale reference is still there while DXGI is
  validating the HWND;
- and `insert_swap_chain` **ignores** a swapchain that differs from the stored one, so once a
  stale reference is in there nothing can displace it.

The fix is to hold nothing across those calls. hudhook already knows how to rebuild itself from
zero on the next `Present`, so releasing early costs one re-initialization and no state.

## The changes, all in `src/hooks/dx12.rs`

1. **`InitializationContext::insert_swap_chain`** replaces the stored swapchain when the incoming
   one has a different `IUnknown` identity, instead of ignoring it. From `Complete` it drops to
   `WithSwapChain`, because the captured command queue belonged to the old swapchain. This is the
   backstop: a stale reference cannot survive a recreation hudhook did not see.

2. **`InitializationContext::holds_swap_chain` / `InitState::holds_swap_chain`** (new): whether a
   strong swapchain reference is currently stored. `Empty` and `Done` hold none.

3. **`same_swap_chain`** (new, beside the existing `identity_ptr`): compares two swapchains by
   their `IUnknown` identity, treating a failed query as "different".

4. **`holds_swap_chain_references`** (new): whether `PIPELINE`, `ACTIVE_CONTEXT` or `INIT_STATE`
   holds anything. **`release_swap_chain_references(operation)`** (new): a `debug!` line and
   `reset_pipeline(operation)` when it does, and nothing at all when it does not - so the
   pre-existing `warn!` inside `reset_pipeline` is not emitted for a reset that had nothing to
   reset.

5. **`dxgi_swap_chain_resize_buffers_impl`, `dxgi_swap_chain_resize_buffers1_impl`,
   `dxgi_swap_chain_set_source_size_impl`** call `release_swap_chain_references` right after the
   existing `wait_for_pipeline_idle_before` and before the trampoline. The post-call
   `update_display_size_after_swap_chain_change` is untouched; it already returns early when
   `PIPELINE` is unset, which after the reset it is, so it becomes a no-op and the next `Present`
   re-initializes through the ordinary path.

6. **`dxgi_factory_create_swap_chain_impl` and `dxgi_factory_create_swap_chain_for_hwnd_impl`**
   call `release_swap_chain_references("swap-chain creation")` before the trampoline, so no
   reference of ours exists while DXGI validates the HWND. Their post-call
   `handle_swap_chain_created` is untouched. These hooks are not reentrant with hudhook's own
   startup: `get_target_addrs` creates its throwaway swapchain while the hooks are only created,
   not yet enabled.

Nothing outside `src/hooks/dx12.rs` was modified by items 1 to 6; item 7 changes
`src/renderer/pipeline.rs`, and item 8 adds `src/output.rs` and touches `src/lib.rs`,
`src/hooks/dx12.rs` and `src/renderer/backend/dx12.rs`. `Cargo.toml` is the published manifest with the
examples, the integration tests, the dev-dependencies they needed and `[profile.test]` removed, and
an empty `[workspace]` table added so the package stays out of the Crimson Desert workspace.
`examples/`, `tests/`, `hudbook/`, `CHANGELOG.md`, `README.md`, `Cargo.lock`,
`rust-toolchain.toml` and `rustfmt.toml` are not vendored. `LICENSE`, `build.rs`, `src/` and
`vendor/minhook/` are as published.

## Notes for whoever revisits this

- **Deadlock.** `release_swap_chain_references` runs on the thread that is calling `ResizeBuffers`
  or `CreateSwapChain`, which is the same thread that calls `Present`, so no render is in flight
  and no lock is contended. `wait_for_pipeline_idle` has already zeroed every frame context's
  fence value by the time the pipeline is dropped, so `D3D12RenderEngine::drop` (which waits on
  any non-zero fence value with an infinite timeout) has nothing left to wait for. In the
  create-swapchain hooks there is no preceding `wait_idle`, so the drop is where the wait happens;
  it waits on the engine's own fence, signalled by the engine's own command queue, which the
  pending swapchain call does not block.
- **WndProc.** `reset_pipeline` moves the render loop out with `Pipeline::take`, which calls
  `Pipeline::cleanup`: `SetWindowLongPtrW(hwnd, GWLP_WNDPROC, shared_state.wnd_proc)` restores the
  window's original procedure and the entry is removed from `PIPELINE_STATES`. `Pipeline` has no
  `Drop` impl, so `take` is the only thing that restores it - dropping a `Pipeline` any other way
  would leave `pipeline_wnd_proc` installed with no state behind it. Every path that disposes of a
  pipeline in this file goes through `reset_pipeline` or `Hooks::unhook`, both of which use
  `take`.
- **ReShade.** The user runs ReShade 6.8 as `bin64/dxgi.dll` with the RenoDX HDR addon. ReShade
  proxies the DXGI factory, the swapchain and `ID3D12CommandQueue`; hudhook's MinHook patches sit
  on the real DXGI/D3D12 vtable functions underneath, so hudhook sees the real objects. That
  matters for `InitializationContext::check_command_queue`, which is untouched here: it probes the
  swapchain's private memory for a pointer to the queue it saw in `ExecuteCommandLists`, and with
  ReShade in the way that queue can be a proxy the real swapchain does not point at. The path that
  actually completes initialization here is the create-swapchain capture (`set_complete`, from
  `handle_swap_chain_created`), which takes the queue from the `IUnknown` the game passed to
  `CreateSwapChainForHwnd`. So a reset that is *not* followed by a swapchain creation - a plain
  successful `ResizeBuffers` - may have to wait for `check_command_queue` to match again before
  the overlay comes back.
- **Not fixed.** The hooks still hold short-lived local references across the trampoline:
  `resize_buffers` casts to `IDXGISwapChain3` before the call, and `resize_buffers1` clones
  `p_this` because it needs it afterwards. Those are references to the swapchain, not to its back
  buffers, and `ResizeBuffers` validates outstanding *back buffer* references, so they are
  harmless - but if a future failure points at them, they can both be moved after the call.

## 7. `renderer/pipeline.rs`: framebuffer scale pinned to 1.0

`update_display_size_from_swap_chain` used to set `io.display_framebuffer_scale` to
`GetDpiForWindow(hwnd) / 96`. The display size it sets in the same breath is the swapchain's back
buffer size, so the framebuffer scale is 1.0 by definition; the DX12 engine multiplies the
viewport by that scale but not the scissor rects, so at any Windows scaling other than 100% the
UI drew enlarged and clipped to `1/scale` of itself (seen in game at 125%: the window content cut
off at 80% of its width and height). Now `[1.0, 1.0]`, and the `GetDpiForWindow` import is gone.
DPI-based readability scaling is done by the render loop (`io.font_global_scale` and
`Style::scale_all_sizes`), where it belongs.

## 8. Output colour space conversion in the DX12 renderer

Crimson Desert presents in HDR. `bin64/ReShade.log` has the back buffer at
`DXGI_FORMAT_R10G10B10A2_UNORM`, and the game (through the RenoDX addon) calls
`IDXGISwapChain3::SetColorSpace1(ColorSpace = 12)`, which is
`DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020` - HDR10, BT.2020 primaries with the PQ transfer
curve. hudhook's pixel shader returned `input.col * texture0.Sample(...)` unchanged, so imgui's
sRGB-encoded colours went into a PQ buffer as if they were PQ. The menu came out blown out and
oversaturated. ReShade's own overlay looks right because its imgui shader converts to the
swapchain's colour space at a paper white of 203 nits (`ReShade.ini`, `HdrOverlayBrightness=203`),
which is what this reproduces.

Three parts, in `src/hooks/dx12.rs`, `src/renderer/backend/dx12.rs` and the new `src/output.rs`.

**Tracking the colour space** (`src/hooks/dx12.rs`). `IDXGISwapChain3::SetColorSpace1` is hooked
the same way `SetSourceSize` and `ResizeBuffers1` are: `get_target_addrs` takes the function from
the throwaway swapchain's vtable by **naming the field** on the `windows` crate's
`IDXGISwapChain3_Vtbl` (`swap_chain3.vtable().SetColorSpace1`), so the crate's own declaration
order picks the slot and no index is typed anywhere. For the record, that slot is 38: the bases
contribute IUnknown 3, `IDXGIObject` 4, `IDXGIDeviceSubObject` 1, `IDXGISwapChain` 10,
`IDXGISwapChain1` 11 and `IDXGISwapChain2` 7, which is 36, and `IDXGISwapChain3` adds
`GetCurrentBackBufferIndex`, `CheckColorSpaceSupport`, `SetColorSpace1`, `ResizeBuffers1` in that
order - one before `ResizeBuffers1`'s 39, which upstream already hooks.

`dxgi_swap_chain_set_color_space1_impl` calls the trampoline first and records nothing when it
fails: a refused colour space leaves the swapchain in the one it already had. On success
`record_color_space` stores the value in `COLOR_SPACE`, a `Mutex<Option<ColorSpaceRecord>>` of
`{ swap_chain_identity, format, color_space }`.

The record keeps the swapchain's **`IUnknown` identity as a `usize`, never a reference**. A strong
reference here would resurrect exactly the bug items 1 to 6 fix, since DXGI refuses to create a
swapchain for an HWND whose previous one is still referenced. `same_swap_chain` compares two live
references by that same identity (`identity_ptr` on the `IUnknown` cast); a record has only the
number left, so it compares the number, and `ActiveDx12Context::swap_chain_identity` already does
the same thing for the same reason.

`effective_color_space(swap_chain, format)` is what `render` consults each frame, right after the
`GetDesc` it already does. It returns the record when both the identity and the format still match,
and otherwise DXGI's own default for a swapchain nobody called `SetColorSpace1` on: scRGB
(`RGB_FULL_G10_NONE_P709`) for an `R16G16B16A16_FLOAT` back buffer, sRGB (`RGB_FULL_G22_NONE_P709`)
for everything else. So a swapchain that was replaced, or resized to a new format, falls back to
the default on its own, which is what the swapchain itself does. A record that no longer matches is
dropped rather than kept: an identity is a raw pointer and a later allocation can be handed the
same address. `Hooks::unhook` clears it with the rest of the state.

**The knobs** (`src/output.rs`, new, `pub mod output`). Plain atomics, so the render thread reads
them once a frame with no lock and a host writes them from any thread:
`set_paper_white_nits`/`paper_white_nits` (default `DEFAULT_PAPER_WHITE_NITS` = 203.0, the ITU-R
BT.2408 diffuse-white level; a value that is not finite and positive reads back as the default),
`set_color_space_override`/`color_space_override` (`ColorSpaceOverride::{Auto, Sdr, Hdr10, ScRgb}`,
`Auto` by default), and `set_detected_color_space`/`detected_color_space`, which the hooks write.
`set_detected_color_space` logs at **info** level when the value actually changes and says nothing
on the frames it does not. `shader_mode()` resolves the override against the detected value into
the `mode` the shader branches on. An unrecognised colour space is SDR, which is the mode that
leaves the pixels exactly as hudhook has always written them.

**The conversion** (`src/renderer/backend/dx12.rs`). The root signature gains a third parameter:
two 32-bit root constants at `b1`, pixel-visible, holding `uint mode` and `float paper_white`. Root
constants rather than a constant buffer so the values can change per frame with no allocation and
no upload; `setup_render_state` writes them next to the projection matrix it already writes. The
pixel shader branches on `mode`, per channel on `rgb` only and never on alpha:

- `0`, SDR: return the colour untouched. Byte for byte what the shader did before, so nothing
  changes for an SDR swapchain.
- `2`, scRGB: sRGB EOTF (`c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)`), then
  `* (paper_white / 80.0)`, 80 nits being the scRGB unit.
- `1`, HDR10: the same sRGB EOTF, then the linear BT.709 to BT.2020 matrix
  (`0.6274 0.3293 0.0433 / 0.0691 0.9195 0.0114 / 0.0164 0.0880 0.8956`), then
  `* (paper_white / 10000.0)` and the ST 2084 inverse EOTF
  (`m1 = 0.1593017578125`, `m2 = 78.84375`, `c1 = 0.8359375`, `c2 = 18.8515625`, `c3 = 18.6875`),
  with the value saturated to `0..1` before the first `pow`.

Blending stays in output space. That is not strictly correct for PQ, and it is what ReShade's
overlay does too; fixing it would mean a second render target and a resolve pass for an overlay
nobody looks at through a transparency gradient.

## 9. `hooks/dx12.rs`: pipeline reset logged at info

`reset_pipeline` logged `Resetting DX12 pipeline: <reason>` at `warn`. With ReShade in the chain
the game resizes and recreates its swapchain once at every launch, so the line appeared in every
session and read as a problem. It is now `info`; the overlay's log shows warnings only for things
that need a look.
