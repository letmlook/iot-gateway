<script setup>
defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
})

defineEmits(['refresh'])
</script>

<template>
  <div class="page-header">
    <div>
      <h1 class="page-header-title">{{ title }}</h1>
      <p v-if="subtitle" class="page-header-subtitle">{{ subtitle }}</p>
    </div>
    <div class="page-header-actions">
      <slot name="actions" />
    </div>
  </div>
</template>
