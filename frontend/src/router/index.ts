import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', name: 'login', component: () => import('../views/LoginView.vue') },
    { path: '/register', name: 'register', component: () => import('../views/RegisterView.vue') },
    {
      path: '/workflows',
      name: 'workflows',
      component: () => import('../views/WorkflowListView.vue'),
      meta: { requiresAuth: true },
    },
    {
      path: '/workflows/:id',
      name: 'workflow-editor',
      component: () => import('../views/WorkflowEditorView.vue'),
      meta: { requiresAuth: true },
    },
    { path: '/', redirect: '/workflows' },
  ],
})

router.beforeEach((to) => {
  const auth = useAuthStore()
  if (to.meta.requiresAuth && !auth.isAuthenticated) {
    return { name: 'login' }
  }
  return true
})

export default router
