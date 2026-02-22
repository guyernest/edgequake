import { Authenticator } from '@aws-amplify/ui-react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { BrowserRouter, Routes, Route } from 'react-router-dom';
import { Toaster } from 'sonner';
import { LogOut } from 'lucide-react';
import { Button } from '@/components/ui/button';
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
      {({ signOut, user }) => (
        <QueryClientProvider client={queryClient}>
          <BrowserRouter>
            <header className="flex items-center justify-between border-b px-6 py-3">
              <a href="/" className="text-sm font-semibold tracking-tight">
                EdgeQuake
              </a>
              <div className="flex items-center gap-3">
                <span className="text-xs text-muted-foreground">
                  {user?.signInDetails?.loginId}
                </span>
                <Button variant="ghost" size="sm" onClick={signOut}>
                  <LogOut className="mr-1.5 h-4 w-4" />
                  Sign out
                </Button>
              </div>
            </header>
            <Routes>
              <Route path="/" element={<DashboardPage />} />
              <Route path="/ns/:namespace" element={<NamespacePage />} />
            </Routes>
            <Toaster richColors position="top-right" />
          </BrowserRouter>
        </QueryClientProvider>
      )}
    </Authenticator>
  );
}
