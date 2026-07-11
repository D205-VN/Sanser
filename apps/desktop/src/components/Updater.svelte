<script lang="ts">
  import { onMount } from 'svelte';
  import { check, type Update } from '@tauri-apps/plugin-updater';
  import { relaunch } from '@tauri-apps/plugin-process';

  let updateAvailable = $state(false);
  let manifest = $state<Update | null>(null);
  let updateStatus = $state<'idle' | 'checking' | 'downloading' | 'installing' | 'done' | 'error'>('idle');
  let progress = $state(0);
  let totalSize = $state(0);
  let errorMessage = $state('');

  onMount(() => {
    // Chờ 5 giây sau khi mở app để kiểm tra cập nhật (tránh làm chậm ứng dụng khi khởi động)
    setTimeout(checkForUpdates, 5000);
  });

  async function checkForUpdates() {
    updateStatus = 'checking';
    try {
      const update = await check();
      // Nếu update khác null nghĩa là có bản cập nhật mới
      if (update) {
        manifest = update;
        updateAvailable = true;
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Failed to check for updates:', msg);
    } finally {
      updateStatus = 'idle';
    }
  }

  async function startUpdate() {
    if (!manifest) return;
    updateStatus = 'downloading';
    progress = 0;
    
    try {
      // Bắt đầu quá trình tải xuống và cài đặt với hàm callback được định kiểu rõ ràng
      await manifest.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            totalSize = event.data.contentLength || 0;
            updateStatus = 'downloading';
            break;
          case 'Progress':
            if (totalSize > 0) {
              progress = Math.round((event.data.chunkLength / totalSize) * 100);
            }
            break;
          case 'Finished':
            updateStatus = 'installing';
            break;
        }
      });
      
      updateStatus = 'done';
      // Relaunch ứng dụng sau 1.5 giây
      setTimeout(async () => {
        try {
          await relaunch();
        } catch {
          // Fallback nếu relaunch bị lỗi
          window.location.reload();
        }
      }, 1500);
    } catch (err: unknown) {
      updateStatus = 'error';
      errorMessage = err instanceof Error ? err.message : 'Có lỗi xảy ra trong quá trình cập nhật.';
    }
  }

  function dismiss() {
    updateAvailable = false;
  }
</script>

{#if updateAvailable && manifest}
  <div class="updater-overlay">
    <div class="updater-modal">
      <div class="updater-header">
        <h3>🚀 Cập nhật ứng dụng</h3>
        <span class="version-tag">v{manifest.version}</span>
      </div>

      <div class="updater-body">
        {#if updateStatus === 'idle'}
          <p class="description">Phiên bản mới của <strong>Sanser</strong> đã sẵn sàng cài đặt. Cập nhật để trải nghiệm những tính năng mới nhất và sửa lỗi.</p>
          {#if manifest.body}
            <div class="changelog">
              <strong>Có gì mới:</strong>
              <p>{manifest.body}</p>
            </div>
          {/if}
        {:else if updateStatus === 'downloading'}
          <p class="status-text">Đang tải bản cập nhật... {progress}%</p>
          <div class="progress-bar-container">
            <div class="progress-bar-fill" style="width: {progress}%"></div>
          </div>
        {:else if updateStatus === 'installing'}
          <p class="status-text">Đang tiến hành giải nén và cài đặt bản cập nhật...</p>
          <div class="spinner"></div>
        {:else if updateStatus === 'done'}
          <p class="status-text success">✨ Cập nhật thành công! Ứng dụng đang khởi động lại...</p>
        {:else if updateStatus === 'error'}
          <p class="status-text error">❌ Cập nhật thất bại: {errorMessage}</p>
        {/if}
      </div>

      <div class="updater-footer">
        {#if updateStatus === 'idle'}
          <button class="btn btn-secondary" onclick={dismiss}>Để sau</button>
          <button class="btn btn-primary" onclick={startUpdate}>Cập nhật ngay</button>
        {:else if updateStatus === 'error'}
          <button class="btn btn-primary" onclick={dismiss}>Đóng</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .updater-overlay {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(0, 0, 0, 0.6);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 9999;
    animation: fadeIn 0.2s ease-out;
  }

  .updater-modal {
    background: #1e1e24;
    border: 1px solid #333;
    border-radius: 12px;
    width: 440px;
    padding: 24px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    gap: 16px;
    color: #e0e0e0;
  }

  .updater-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid #333;
    padding-bottom: 12px;
  }

  .updater-header h3 {
    margin: 0;
    font-size: 1.2rem;
    font-weight: 600;
  }

  .version-tag {
    background: #007acc;
    color: white;
    padding: 2px 8px;
    border-radius: 12px;
    font-size: 0.8rem;
    font-weight: 500;
  }

  .updater-body {
    font-size: 0.9rem;
    line-height: 1.5;
  }

  .changelog {
    background: #121214;
    border-radius: 6px;
    padding: 10px;
    margin-top: 10px;
    max-height: 100px;
    overflow-y: auto;
  }

  .progress-bar-container {
    width: 100%;
    height: 8px;
    background: #333;
    border-radius: 4px;
    margin-top: 8px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background: #007acc;
    transition: width 0.2s ease-out;
  }

  .updater-footer {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
    border-top: 1px solid #333;
    padding-top: 16px;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: 0.9rem;
    cursor: pointer;
    font-weight: 500;
    transition: all 0.2s ease;
  }

  .btn-primary {
    background: #007acc;
    color: white;
  }

  .btn-primary:hover {
    background: #0098ff;
  }

  .btn-secondary {
    background: #2a2a30;
    color: #a0a0a0;
  }

  .btn-secondary:hover {
    background: #3a3a40;
    color: #e0e0e0;
  }

  .status-text {
    margin: 8px 0;
  }

  .status-text.success {
    color: #4caf50;
  }

  .status-text.error {
    color: #f44336;
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 3px solid #333;
    border-top: 3px solid #007acc;
    border-radius: 50%;
    animation: spin 1s linear infinite;
    margin: 10px auto;
  }

  @keyframes spin {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
</style>
