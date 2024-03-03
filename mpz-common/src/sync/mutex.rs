//! Synchronized mutex.

use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, MutexGuard},
    task::{ready, Context, Poll, Waker},
};

use futures::{channel::mpsc::Sender, Future, Stream, StreamExt};

use crate::ThreadId;

/// A mutex which synchronizes exclusive access to a resource across logical threads.
///
/// There are two configurations for a mutex, either as a leader or as a follower.
///
/// **Leader**
///
/// A leader mutex is the authority on the order in which threads can acquire a lock. When a
/// thread acquires a lock, it broadcasts a message to all follower mutexes, which then enforce
/// that this order is preserved.
///
/// **Follower**
///
/// A follower mutex waits for messages from the leader mutex to inform it of the order in which
/// threads can acquire a lock.
#[derive(Debug)]
pub struct Mutex<T> {
    inner: MutexInner<T>,
}

impl<T> Mutex<T> {
    /// Creates a new leader mutex.
    ///
    /// # Arguments
    ///
    /// * `value` - The value protected by the mutex.
    /// * `sender` - The sender to broadcast ordering messages.
    pub fn new_leader(value: T, sender: Sender<ThreadId>) -> Self {
        Self {
            inner: MutexInner::Leader(Leader {
                mutex: StdMutex::new(value),
                sender: Arc::new(StdMutex::new(sender)),
            }),
        }
    }

    /// Creates a new follower mutex.
    ///
    /// # Arguments
    ///
    /// * `value` - The value protected by the mutex.
    /// * `stream` - The stream to receive ordering messages.
    pub fn new_follower<St>(value: T, stream: St) -> (Self, MutexBroker<St>) {
        let queue = Arc::new(StdMutex::new(Queue {
            next: None,
            ready: None,
            waiting: HashMap::new(),
        }));

        let broker = MutexBroker {
            stream,
            queue: queue.clone(),
        };

        (
            Self {
                inner: MutexInner::Follower(Follower {
                    mutex: StdMutex::new(value),
                    queue,
                }),
            },
            broker,
        )
    }

    /// Returns a future that resolves once a lock has been acquired.
    pub fn lock<'a>(&'a self, id: &'a ThreadId) -> Lock<'a, T> {
        Lock {
            inner: &self.inner,
            id,
        }
    }

    /// Returns the inner value, consuming the mutex.
    pub fn into_inner(self) -> T {
        match self.inner {
            MutexInner::Leader(leader) => leader.mutex.into_inner().unwrap(),
            MutexInner::Follower(follower) => follower.mutex.into_inner().unwrap(),
        }
    }
}

/// Future for the [`lock`](`Mutex::lock`) method.
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Lock<'a, T> {
    inner: &'a MutexInner<T>,
    id: &'a ThreadId,
}

impl<'a, T> Future for Lock<'a, T> {
    type Output = Result<MutexGuard<'a, T>, MutexError>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match &self.inner {
            MutexInner::Leader(leader) => leader.poll_lock(cx, &self.id),
            MutexInner::Follower(follower) => follower.poll_lock(cx, &self.id),
        }
    }
}

#[derive(Debug)]
pub struct MutexError;

#[derive(Debug)]
enum MutexInner<T> {
    Leader(Leader<T>),
    Follower(Follower<T>),
}

#[derive(Debug)]
struct Leader<T> {
    mutex: StdMutex<T>,
    sender: Arc<StdMutex<Sender<ThreadId>>>,
}

impl<T> Leader<T> {
    fn poll_lock(
        &self,
        cx: &mut Context,
        id: &ThreadId,
    ) -> Poll<Result<MutexGuard<'_, T>, MutexError>> {
        let mut sender = self.sender.lock().unwrap();
        ready!(sender.poll_ready(cx)).unwrap();
        sender.start_send(id.clone()).unwrap();

        let guard = self.mutex.lock().unwrap();

        // Make sure to free the sender lock *after* acquiring the mutex lock.
        // Otherwise a competing thread might budge in line.
        drop(sender);

        Poll::Ready(Ok(guard))
    }
}

#[derive(Debug)]
struct Follower<T> {
    mutex: StdMutex<T>,
    queue: Arc<StdMutex<Queue>>,
}

impl<T> Follower<T> {
    fn poll_lock(
        &self,
        cx: &mut Context,
        id: &ThreadId,
    ) -> Poll<Result<MutexGuard<'_, T>, MutexError>> {
        let mut queue = self.queue.lock().unwrap();
        if queue.next.as_ref().map(|next| next == id).unwrap_or(false) {
            queue.next.take();
            queue.ready.take().map(|waker| waker.wake());
            let guard = self.mutex.lock().unwrap();

            // Make sure to free the queue lock *after* acquiring the mutex lock.
            // Otherwise a competing thread might budge in line.
            drop(queue);

            Poll::Ready(Ok(guard))
        } else {
            queue.waiting.insert(id.clone(), cx.waker().clone());
            Poll::Pending
        }
    }
}

#[derive(Debug)]
struct Queue {
    next: Option<ThreadId>,
    ready: Option<Waker>,
    waiting: HashMap<ThreadId, Waker>,
}

/// A broker for a follower mutex.
///
/// A broker is a future which forwards messages from the leader mutex to the follower mutex.
/// It must be polled continuously in order to make progress, and resolves when the stream of
/// messages is exhausted.
#[derive(Debug)]
pub struct MutexBroker<T> {
    stream: T,
    queue: Arc<StdMutex<Queue>>,
}

impl<T> MutexBroker<T> {
    /// Returns whether the queue is ready to accept the next lock request.
    fn is_ready(&self) -> bool {
        self.queue.lock().unwrap().next.is_none()
    }

    /// Sets the next thread to acquire the lock.
    fn set_next(&self, cx: &mut Context<'_>, id: ThreadId) {
        let mut queue = self.queue.lock().unwrap();
        // Wake up the waiting thread.
        queue.waiting.remove(&id).map(|waker| waker.wake());
        // Set the next thread to acquire the lock.
        queue.next = Some(id);
        // Set the ready waker.
        queue.ready = Some(cx.waker().clone());
    }
}

impl<T> Future for MutexBroker<T>
where
    T: Stream<Item = ThreadId> + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.is_ready() {
            return Poll::Pending;
        }

        let Some(next) = ready!(self.stream.poll_next_unpin(cx)) else {
            return Poll::Ready(());
        };

        self.set_next(cx, next);

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc::Receiver;

    use super::*;

    pub fn mutex<T, U>(
        leader_value: T,
        follower_value: U,
        buffer: usize,
    ) -> (Mutex<T>, Mutex<U>, MutexBroker<Receiver<ThreadId>>) {
        let (sender, receiver) = futures::channel::mpsc::channel(buffer);

        let leader_mutex = Mutex::new_leader(leader_value, sender);
        let (follower_mutex, broker) = Mutex::new_follower(follower_value, receiver);

        (leader_mutex, follower_mutex, broker)
    }

    #[test]
    fn test_mutex() {
        let (leader_mutex, follower_mutex, broker) = mutex((), (), 10);

        let leader_mutex = Arc::new(leader_mutex);
        let follower_mutex = Arc::new(follower_mutex);

        let id_0 = ThreadId::new(0);
        let id_1 = ThreadId::new(1);
        let id_2 = ThreadId::new(2);

        futures::executor::block_on(async {
            futures::join!(
                broker,
                async {
                    drop(leader_mutex.lock(&id_0).await.unwrap());
                    drop(leader_mutex.lock(&id_1).await.unwrap());
                    drop(leader_mutex.lock(&id_2).await.unwrap());
                    drop(leader_mutex);
                },
                async { drop(follower_mutex.lock(&id_2).await.unwrap()) },
                async { drop(follower_mutex.lock(&id_1).await.unwrap()) },
                async { drop(follower_mutex.lock(&id_0).await.unwrap()) },
            );
        });
    }

    #[test]
    fn test_follower() {
        let id_0 = ThreadId::new(0);
        let id_1 = ThreadId::new(1);

        let (mutex, broker) =
            Mutex::new_follower((), futures::stream::iter(vec![id_0.clone(), id_1.clone()]));

        let mutex = Arc::new(mutex);

        futures::executor::block_on(async {
            futures::join!(
                broker,
                async {
                    drop(mutex.clone().lock(&id_1).await.unwrap());
                },
                async {
                    drop(mutex.clone().lock(&id_0).await.unwrap());
                },
            );
        });
    }
}
