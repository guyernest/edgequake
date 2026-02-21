import { Authenticator } from '@aws-amplify/ui-react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Routes, Route } from 'react-router-dom';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 1,
    },
  },
});

function DashboardPage() {
  return (
    <div className="p-8">
      <h1 className="text-2xl font-bold">EdgeQuake Console</h1>
      <p className="text-muted-foreground mt-2">Dashboard coming in Plan 02b</p>
    </div>
  );
}

function NamespacePage() {
  return (
    <div className="p-8">
      <h1 className="text-2xl font-bold">Namespace Detail</h1>
      <p className="text-muted-foreground mt-2">Namespace detail coming in Plan 03</p>
    </div>
  );
}

export default function App() {
  return (
    <Authenticator>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <Routes>
            <Route path="/" element={<DashboardPage />} />
            <Route path="/ns/:namespace" element={<NamespacePage />} />
          </Routes>
        </BrowserRouter>
      </QueryClientProvider>
    </Authenticator>
  );
}
