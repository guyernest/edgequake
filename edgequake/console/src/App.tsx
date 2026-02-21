import { Authenticator } from '@aws-amplify/ui-react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Toaster } from 'sonner';
import { DashboardPage } from '@/pages/DashboardPage';
import { NamespacePage } from '@/pages/NamespacePage';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 1,
    },
  },
});

export default function App() {
  return (
    <Authenticator>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <Routes>
            <Route path="/" element={<DashboardPage />} />
            <Route path="/ns/:namespace" element={<NamespacePage />} />
          </Routes>
          <Toaster richColors position="top-right" />
        </BrowserRouter>
      </QueryClientProvider>
    </Authenticator>
  );
}
