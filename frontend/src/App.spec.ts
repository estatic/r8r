import { describe, it, expect, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import App from './App.vue'
import LoginView from './views/LoginView.vue'
import WorkflowListView from './views/WorkflowListView.vue'

describe('App', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
  })

  it('redirects an unauthenticated visitor to the login page', async () => {
    const router = createRouter({
      history: createWebHistory(),
      routes: [
        { path: '/login', name: 'login', component: LoginView },
        { path: '/workflows', name: 'workflows', component: WorkflowListView, meta: { requiresAuth: true } },
        { path: '/', redirect: '/workflows' },
      ],
    })
    router.beforeEach((to) => {
      if (to.meta.requiresAuth) return { name: 'login' }
      return true
    })
    router.push('/')
    await router.isReady()

    const wrapper = mount(App, { global: { plugins: [router] } })
    expect(wrapper.text()).toContain('Log in to r8r')
  })
})
