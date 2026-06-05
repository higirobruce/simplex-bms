"use client";

import { useState, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/ui/pagination";
import { apiCall } from "@/lib/fetcher";
import { useDebounce } from "@/lib/hooks";
import { Search } from "lucide-react";

export default function AdminUsersPage() {
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const debouncedSearch = useDebounce(search, 300);
  useEffect(() => { setPage(1); }, [debouncedSearch]);

  const { data, isLoading } = useQuery({
    queryKey: ["admin-users", page, debouncedSearch],
    queryFn: () =>
      apiCall(`/api/admin/users?page=${page}&limit=25&search=${encodeURIComponent(debouncedSearch)}`),
  });

  return (
    <div className="animate-fade-in">
      <PageHeader
        eyebrow="Platform"
        title="Users"
        description="Everyone with access to a shop, across the whole platform."
      />

      <div className="relative mb-4 max-w-sm">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-ink-mute" strokeWidth={1.75} />
        <Input
          className="pl-9"
          placeholder="Search by name or email…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      <Card>
        <CardContent className="py-2">
          {isLoading ? (
            <div className="py-10 text-center font-mono text-[0.7rem] uppercase tracking-[0.18em] text-ink-mute">
              Loading…
            </div>
          ) : (data?.data?.length ?? 0) === 0 ? (
            <div className="py-10 text-center text-sm text-ink-mute">No users found.</div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Email</TableHead>
                  <TableHead>Shop</TableHead>
                  <TableHead>Role</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.data.map((u: any) => (
                  <TableRow key={u.id}>
                    <TableCell className="font-medium text-ink">{u.name || "—"}</TableCell>
                    <TableCell className="text-ink-soft">{u.email}</TableCell>
                    <TableCell>
                      {u.org ? (
                        <span className="inline-flex items-center gap-2">
                          <span className="text-ink-soft">{u.org.name}</span>
                          {u.org.status === "SUSPENDED" && (
                            <Badge variant="destructive">suspended</Badge>
                          )}
                        </span>
                      ) : (
                        "—"
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge variant={u.role === "ADMIN" ? "default" : "secondary"}>
                        {u.role.toLowerCase()}
                      </Badge>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <Pagination page={page} totalPages={data?.meta?.pages || 1} onPageChange={setPage} />
    </div>
  );
}
