<script lang="ts">
  let {
    checked,
    disabled = false,
    label,
    description,
    onchange
  }: {
    checked: boolean;
    disabled?: boolean;
    label: string;
    description: string;
    onchange: (checked: boolean) => void;
  } = $props();
</script>

<div class="toggle-row">
  <div class="toggle-copy"><strong>{label}</strong><span>{description}</span></div>
  <button
    type="button"
    class="switch"
    class:on={checked}
    role="switch"
    aria-checked={checked}
    aria-label={label}
    {disabled}
    onclick={() => onchange(!checked)}
  ><span></span></button>
</div>
