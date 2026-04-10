import { type ClassValue, clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

/** Merge Tailwind CSS class names, resolving conflicts so the last class wins. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}