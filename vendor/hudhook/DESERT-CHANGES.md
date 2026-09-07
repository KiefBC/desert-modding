# Changes against hudhook 0.9.2

This is the published [hudhook](https://github.com/veeenu/hudhook) 0.9.2 crate (MIT) with the
changes below applied. It is patched into the workspace by `[patch.crates-io]` in the root
`Cargo.toml`; nothing else in the tree depends on it. Everything here is meant to be upstreamable
as it stands, and to be re-applicable to a later hudhook by hand: it is six small hunks in one
file.

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

Nothing outside `src/hooks/dx12.rs` was modified. `Cargo.toml` is the published manifest with the
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
