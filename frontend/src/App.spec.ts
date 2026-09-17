import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import App from './App.vue'

describe('App', () => {
  it('renders the r8r heading', () => {
    const wrapper = mount(App)
    expect(wrapper.text()).toContain('r8r')
  })
})
